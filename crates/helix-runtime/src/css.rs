//! Task: Integrate `lightningcss` + `selectors` (Servo) for CSS parsing and
//! rule matching.
//!
//! `lightningcss` parses stylesheet text into CSS rules/declarations; the
//! `selectors` crate (the same crate Servo/Stylo uses) is used independently
//! to parse each rule's selector text and match it against DOM elements.
//! `lightningcss` has its own internal selector representation
//! (`parcel_selectors`) used for its own serialization/minification, but we
//! re-parse selector text with `selectors` here so matching can run directly
//! against our `html5ever`/`markup5ever_rcdom` tree.

use std::borrow::Borrow;
use std::fmt;

use cssparser::{ParseError, Parser as CssParser, ParserInput, ToCss};
use lightningcss::rules::CssRule;
use lightningcss::stylesheet::{ParserOptions, StyleSheet};
use lightningcss::traits::ToCss as LightningToCss;
use markup5ever_rcdom::{Handle, NodeData};
use selectors::attr::{AttrSelectorOperation, CaseSensitivity, NamespaceConstraint};
use selectors::bloom::BloomFilter;
use selectors::context::SelectorCaches;
use selectors::matching::{
    ElementSelectorFlags, MatchingContext, MatchingMode, NeedsSelectorFlags, QuirksMode,
};
use selectors::parser::{
    NonTSPseudoClass as NonTSPseudoClassTrait, ParseRelative, PseudoElement as PseudoElementTrait,
    Selector, SelectorImpl, SelectorList, SelectorParseErrorKind,
};
use selectors::{Element, OpaqueElement};

/// An interned-string stand-in used for every string-like associated type
/// `selectors::parser::SelectorImpl` requires (identifiers, local names,
/// namespaces, attribute values). A production implementation would reuse
/// `html5ever`'s `string_cache` atoms; a plain wrapper keeps this crate's
/// first CSS-matching pass simple.
#[derive(Clone, Debug, Default, Eq, PartialEq, Hash)]
pub struct Atom(pub String);

impl From<&str> for Atom {
    fn from(value: &str) -> Self {
        Atom(value.to_owned())
    }
}

impl Borrow<str> for Atom {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for Atom {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl precomputed_hash::PrecomputedHash for Atom {
    fn precomputed_hash(&self) -> u32 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.0.hash(&mut hasher);
        hasher.finish() as u32
    }
}

impl ToCss for Atom {
    fn to_css<W: fmt::Write>(&self, dest: &mut W) -> fmt::Result {
        dest.write_str(&self.0)
    }
}

/// This runtime does not yet support any non-tree-structural pseudo-classes
/// (`:hover`, `:focus`, ...); the empty enum means the parser rejects them
/// with a parse error rather than silently mis-matching.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NoPseudoClass {}

impl ToCss for NoPseudoClass {
    fn to_css<W: fmt::Write>(&self, _dest: &mut W) -> fmt::Result {
        match *self {}
    }
}

impl NonTSPseudoClassTrait for NoPseudoClass {
    type Impl = HelixSelectorImpl;

    fn is_active_or_hover(&self) -> bool {
        match *self {}
    }

    fn is_user_action_state(&self) -> bool {
        match *self {}
    }
}

/// No pseudo-elements (`::before`, `::after`, ...) are supported yet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NoPseudoElement {}

impl ToCss for NoPseudoElement {
    fn to_css<W: fmt::Write>(&self, _dest: &mut W) -> fmt::Result {
        match *self {}
    }
}

impl PseudoElementTrait for NoPseudoElement {
    type Impl = HelixSelectorImpl;
}

/// The `selectors::parser::SelectorImpl` binding this runtime's selector
/// matching to the [`Atom`] string type and the `html5ever`/
/// `markup5ever_rcdom` DOM (via [`DomElement`]).
#[derive(Clone, Debug)]
pub struct HelixSelectorImpl;

impl SelectorImpl for HelixSelectorImpl {
    type ExtraMatchingData<'a> = ();
    type AttrValue = Atom;
    type Identifier = Atom;
    type LocalName = Atom;
    type NamespaceUrl = Atom;
    type NamespacePrefix = Atom;
    type BorrowedNamespaceUrl = str;
    type BorrowedLocalName = str;
    type NonTSPseudoClass = NoPseudoClass;
    type PseudoElement = NoPseudoElement;
}

/// A `selectors::parser::Parser` for [`HelixSelectorImpl`]. Every method has
/// a usable default (reject pseudo-classes/elements, disallow `:is`/`:where`/
/// `:has`), so no overrides are needed yet.
pub struct HelixParser;

impl<'i> selectors::parser::Parser<'i> for HelixParser {
    type Impl = HelixSelectorImpl;
    type Error = SelectorParseErrorKind<'i>;
}

/// Parses a comma-separated selector list (e.g. `"div.card > p, #main"`)
/// using the Servo `selectors` crate.
pub fn parse_selector_list(
    selector_text: &str,
) -> Result<SelectorList<HelixSelectorImpl>, ParseError<'_, SelectorParseErrorKind<'_>>> {
    let mut input = ParserInput::new(selector_text);
    let mut parser = CssParser::new(&mut input);
    SelectorList::parse(&HelixParser, &mut parser, ParseRelative::No)
}

/// One CSS rule reduced to what this crate currently needs for matching:
/// its parsed selectors and its raw declaration-block text.
pub struct StyleRule {
    pub selectors: SelectorList<HelixSelectorImpl>,
    pub declarations_css: String,
}

/// Parses stylesheet source with `lightningcss` and re-parses each style
/// rule's selector text with `selectors`, discarding rules whose selectors
/// this runtime doesn't support yet (e.g. pseudo-classes).
pub fn parse_stylesheet(source: &str) -> Vec<StyleRule> {
    // Malformed CSS must not take down the renderer: `lightningcss` returns an
    // error rather than panicking, and we degrade gracefully to "no rules" so a
    // bad stylesheet simply contributes no styling.
    let Ok(stylesheet) = StyleSheet::parse(source, ParserOptions::default()) else {
        return Vec::new();
    };

    let mut rules = Vec::new();
    for rule in &stylesheet.rules.0 {
        if let CssRule::Style(style_rule) = rule {
            let selector_text = style_rule
                .selectors
                .to_css_string(Default::default())
                .unwrap_or_default();
            if let Ok(selectors) = parse_selector_list(&selector_text) {
                rules.push(StyleRule {
                    selectors,
                    declarations_css: style_rule
                        .declarations
                        .to_css_string(Default::default())
                        .unwrap_or_default(),
                });
            }
        }
    }
    rules
}

/// A `selectors::Element` implementation over a `markup5ever_rcdom` node,
/// enabling `selectors`-based rule matching directly against the tree built
/// by [`crate::html::parse_html`].
#[derive(Clone)]
pub struct DomElement(pub Handle);

impl fmt::Debug for DomElement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DomElement").field("name", &self.local_name()).finish()
    }
}

impl DomElement {
    fn local_name(&self) -> Option<String> {
        match &self.0.data {
            NodeData::Element { name, .. } => Some(name.local.to_string()),
            _ => None,
        }
    }

    fn parent_handle(&self) -> Option<Handle> {
        self.0
            .parent
            .take()
            .inspect(|weak| self.0.parent.set(Some(weak.clone())))
            .and_then(|weak| weak.upgrade())
    }

    fn attr(&self, name: &str) -> Option<String> {
        match &self.0.data {
            NodeData::Element { attrs, .. } => attrs
                .borrow()
                .iter()
                .find(|a| &*a.name.local == name)
                .map(|a| a.value.to_string()),
            _ => None,
        }
    }
}

impl Element for DomElement {
    type Impl = HelixSelectorImpl;

    fn opaque(&self) -> OpaqueElement {
        OpaqueElement::new(&*self.0)
    }

    fn parent_element(&self) -> Option<Self> {
        self.parent_handle()
            .filter(|h| matches!(h.data, NodeData::Element { .. }))
            .map(DomElement)
    }

    fn parent_node_is_shadow_root(&self) -> bool {
        false
    }

    fn containing_shadow_host(&self) -> Option<Self> {
        None
    }

    fn is_pseudo_element(&self) -> bool {
        false
    }

    fn prev_sibling_element(&self) -> Option<Self> {
        let parent = self.parent_handle()?;
        let siblings = parent.children.borrow();
        let idx = siblings.iter().position(|c| std::rc::Rc::ptr_eq(c, &self.0))?;
        siblings[..idx]
            .iter()
            .rev()
            .find(|c| matches!(c.data, NodeData::Element { .. }))
            .map(|h| DomElement(h.clone()))
    }

    fn next_sibling_element(&self) -> Option<Self> {
        let parent = self.parent_handle()?;
        let siblings = parent.children.borrow();
        let idx = siblings.iter().position(|c| std::rc::Rc::ptr_eq(c, &self.0))?;
        siblings[idx + 1..]
            .iter()
            .find(|c| matches!(c.data, NodeData::Element { .. }))
            .map(|h| DomElement(h.clone()))
    }

    fn first_element_child(&self) -> Option<Self> {
        self.0
            .children
            .borrow()
            .iter()
            .find(|c| matches!(c.data, NodeData::Element { .. }))
            .map(|h| DomElement(h.clone()))
    }

    fn is_html_element_in_html_document(&self) -> bool {
        true
    }

    fn has_local_name(&self, local_name: &str) -> bool {
        self.local_name().as_deref() == Some(local_name)
    }

    fn has_namespace(&self, ns: &str) -> bool {
        match &self.0.data {
            NodeData::Element { name, .. } => &*name.ns == ns,
            _ => false,
        }
    }

    fn is_same_type(&self, other: &Self) -> bool {
        self.local_name() == other.local_name()
    }

    fn attr_matches(
        &self,
        _ns: &NamespaceConstraint<&Atom>,
        local_name: &Atom,
        operation: &AttrSelectorOperation<&Atom>,
    ) -> bool {
        match self.attr(&local_name.0) {
            Some(value) => operation.eval_str(&value),
            None => false,
        }
    }

    fn match_non_ts_pseudo_class(
        &self,
        pc: &NoPseudoClass,
        _context: &mut MatchingContext<Self::Impl>,
    ) -> bool {
        match *pc {}
    }

    fn match_pseudo_element(
        &self,
        pe: &NoPseudoElement,
        _context: &mut MatchingContext<Self::Impl>,
    ) -> bool {
        match *pe {}
    }

    fn apply_selector_flags(&self, _flags: ElementSelectorFlags) {}

    fn is_link(&self) -> bool {
        self.local_name().as_deref() == Some("a") && self.attr("href").is_some()
    }

    fn is_html_slot_element(&self) -> bool {
        false
    }

    fn has_id(&self, id: &Atom, case_sensitivity: CaseSensitivity) -> bool {
        self.attr("id")
            .is_some_and(|v| case_sensitivity.eq(v.as_bytes(), id.0.as_bytes()))
    }

    fn has_class(&self, name: &Atom, case_sensitivity: CaseSensitivity) -> bool {
        self.attr("class").is_some_and(|classes| {
            classes
                .split_ascii_whitespace()
                .any(|c| case_sensitivity.eq(c.as_bytes(), name.0.as_bytes()))
        })
    }

    fn has_custom_state(&self, _name: &Atom) -> bool {
        false
    }

    fn imported_part(&self, _name: &Atom) -> Option<Atom> {
        None
    }

    fn is_part(&self, _name: &Atom) -> bool {
        false
    }

    fn is_empty(&self) -> bool {
        !self.0.children.borrow().iter().any(|c| match &c.data {
            NodeData::Element { .. } => true,
            NodeData::Text { contents } => !contents.borrow().is_empty(),
            _ => false,
        })
    }

    fn is_root(&self) -> bool {
        self.parent_handle()
            .is_none_or(|p| !matches!(p.data, NodeData::Element { .. }))
    }

    fn add_element_unique_hashes(&self, _filter: &mut BloomFilter) -> bool {
        false
    }
}

/// Returns whether `element` matches `selector`, using a fresh, default
/// `MatchingContext` (no `:hover`/quirks-mode state tracked yet).
pub fn matches(selector: &Selector<HelixSelectorImpl>, element: &DomElement) -> bool {
    let mut caches = SelectorCaches::default();
    let mut context = MatchingContext::new(
        MatchingMode::Normal,
        None,
        &mut caches,
        QuirksMode::NoQuirks,
        NeedsSelectorFlags::No,
        selectors::matching::MatchingForInvalidation::No,
    );
    selectors::matching::matches_selector(selector, 0, None, element, &mut context)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::parse_html;
    use markup5ever_rcdom::{Handle, NodeData, RcDom};

    fn root_child_element(dom: &markup5ever_rcdom::RcDom) -> DomElement {
        fn find(handle: &Handle) -> Option<Handle> {
            if matches!(handle.data, NodeData::Element { .. }) {
                return Some(handle.clone());
            }
            handle.children.borrow().iter().find_map(find)
        }
        DomElement(find(&dom.document).expect("an element"))
    }

    #[test]
    fn parses_stylesheet_rules() {
        let rules = parse_stylesheet("div.card { color: red; } #main p { color: blue; }");
        assert_eq!(rules.len(), 2);
    }

    #[test]
    fn matches_class_selector() {
        let dom = parse_html(r#"<html><body><div class="card">hi</div></body></html>"#);
        let rules = parse_stylesheet("div.card { color: red; }");
        let rule = &rules[0];

        fn find_div(handle: &Handle) -> Option<Handle> {
            if let NodeData::Element { name, .. } = &handle.data {
                if &*name.local == "div" {
                    return Some(handle.clone());
                }
            }
            handle.children.borrow().iter().find_map(find_div)
        }
        let div = DomElement(find_div(&dom.document).expect("a div"));

        assert!(rule.selectors.slice().iter().any(|s| matches(s, &div)));
    }

    #[test]
    fn does_not_match_wrong_selector() {
        let dom = parse_html("<html><body><p>hi</p></body></html>");
        let rules = parse_stylesheet("div.card { color: red; }");
        let element = root_child_element(&dom);
        assert!(!rules[0]
            .selectors
            .slice()
            .iter()
            .any(|s| matches(s, &element)));
    }

    #[test]
    fn matches_id_selector() {
        let dom = parse_html(r#"<html><body><div id="main">hi</div></body></html>"#);
        let rules = parse_stylesheet("#main { color: red; }");
        assert!(!rules.is_empty(), "id selector should parse");
        let element = root_child_element(&dom);
        assert!(rules[0].selectors.slice().iter().any(|s| matches(s, &element)));
    }

    #[test]
    fn matches_attribute_selector() {
        let dom = parse_html(r#"<html><body><input type="text"><input type="password"></body></html>"#);
        let rules = parse_stylesheet(r#"input[type="password"] { color: red; }"#);
        // Two input elements: find each and assert only the password one matches.
        let mut inputs = Vec::new();
        find_all(&dom.document, "input", &mut inputs);

        let password = inputs
            .iter()
            .find(|el| DomElement(el.clone()).attr("type").as_deref() == Some("password"))
            .expect("password input");
        let text = inputs
            .iter()
            .find(|el| DomElement(el.clone()).attr("type").as_deref() == Some("text"))
            .expect("text input");

        assert!(rules[0]
            .selectors
            .slice()
            .iter()
            .any(|s| matches(s, &DomElement(password.clone()))));
        assert!(!rules[0]
            .selectors
            .slice()
            .iter()
            .any(|s| matches(s, &DomElement(text.clone()))));
    }

    #[test]
    fn matches_descendant_combinator() {
        let dom = parse_html(
            r#"<html><body><section><p>deep</p></section><p>shallow</p></body></html>"#,
        );
        let rules = parse_stylesheet("section p { color: red; }");
        let mut ps = Vec::new();
        find_all(&dom.document, "p", &mut ps);
        assert_eq!(ps.len(), 2);
        let deep = ps[0].clone();
        let shallow = ps[1].clone();
        assert!(rules[0]
            .selectors
            .slice()
            .iter()
            .any(|s| matches(s, &DomElement(deep))));
        assert!(!rules[0]
            .selectors
            .slice()
            .iter()
            .any(|s| matches(s, &DomElement(shallow))));
    }

    #[test]
    fn unsupported_pseudo_class_is_dropped_not_crashed() {
        // `:hover` is not yet supported; the rule is skipped so matching stays
        // sound rather than panicking or silently matching everything.
        let dom = parse_html(r#"<html><body><a href="x">hi</a></body></html>"#);
        let rules = parse_stylesheet("a:hover { color: red; }");
        assert!(rules.is_empty(), "unsupported pseudo-class rules are dropped");
        assert_eq!(tags(&dom), vec!["html", "body", "a"]);
    }

    #[test]
    fn malformed_css_yields_no_rules() {
        // Unbalanced braces / garbage must degrade to no rules, not panic.
        assert!(parse_stylesheet("{ color: red;;; @@@ not css {").is_empty());
        assert!(parse_stylesheet("div { color: }").is_empty());
    }

    fn find_all(handle: &Handle, tag: &str, out: &mut Vec<Handle>) {
        if let NodeData::Element { name, .. } = &handle.data {
            if &*name.local == tag {
                out.push(handle.clone());
            }
        }
        for child in handle.children.borrow().iter() {
            find_all(child, tag, out);
        }
    }

    fn tags(dom: &markup5ever_rcdom::RcDom) -> Vec<String> {
        let mut out = Vec::new();
        fn walk(handle: &Handle, out: &mut Vec<String>) {
            if let NodeData::Element { name, .. } = &handle.data {
                out.push(name.local.to_string());
            }
            for child in handle.children.borrow().iter() {
                walk(child, out);
            }
        }
        walk(&dom.document, &mut out);
        out
    }
}
