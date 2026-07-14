//! Stage S2 (Transpile) — P1 pattern: static content sites.
//!
//! `transpile_static_site` takes a static HTML document (blogs, documentation,
//! marketing pages — the lowest-complexity migration pattern, spec §6.2 P1) and
//! emits a [`TranspiledSite`]: a structured list of [`DomOp`]s plus the
//! generated guest Rust source and a WIT `world` snippet that rebuild the same
//! DOM through the Helix `dom` capability interface (see `wit/helix.wit`).
//!
//! The emitted op list is the contract Stage S3 (Validate) checks for
//! equivalence: the number of element/text operations must match the source
//! tree, and concatenation of text runs must be preserved byte-for-byte.
//!
//! Text-modeling note: the `dom` interface has no dedicated text node, so a
//! text run becomes either `set-text` on its element (when the element has only
//! that text child) or a `<span>` child carrying the text (mixed content). This
//! is a faithful, renderer-agnostic representation of the static content.
//!
//! `transpile_form_app` extends the same pipeline to the **P2 form-based CRUD /
//! dashboard** pattern (spec §6.2 P2). Beyond the static content model it
//! captures *interactive* semantics: form fields (`input`/`select`/`textarea`,
//! with their `name`/`type`), submit/change/input/click event handlers, and
//! tabular entity listings (`table`/`tr`) — the shape a CRUD admin panel takes.
//! The detected structure is surfaced in [`TranspiledSite::crud`] so Stage S3
//! can assert CRUD equivalence (fields bind, submit is wired, rows map 1:1).
//!
//! [`transpile_data_viz`] extends the pipeline to the **P3 data-visualization**
//! pattern (spec §6.2 P3). It records chart mount points — `<canvas>`/`<svg>`
//! elements with their `id`, pixel `width`/`height`, and `data-*` series — in
//! [`TranspiledSite::dataviz`].
//!
//! [`transpile_media_player`] extends the pipeline to the **P4 media-player**
//! pattern (spec §6.2 P4). It records `<video>`/`<audio>` players with their
//! `src`, `controls`/`autoplay`/`loop` hints, and `<source>` alternate streams
//! in [`TranspiledSite::media`].

use std::fmt::Write;

/// A node in the parsed static HTML tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Element(Element),
    /// Non-whitespace text run (whitespace-only runs are dropped on parse).
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    pub tag: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

/// A single DOM operation in the transpiled output, in execution order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomOp {
    /// `let <var> = dom::create_element(&"<tag>");`
    Create { var: String, tag: String },
    /// `dom::set_text(<var>, &"<text>");`
    Text { var: String, text: String },
    /// `dom::set_attribute(<var>, &"<name>", &"<value>");`
    Attr {
        var: String,
        name: String,
        value: String,
    },
    /// `dom::append_child(<parent>, <child>);`
    Append { parent: String, child: String },
    /// `dom::on_click(<var>, <handler>);`
    OnClick { var: String, handler: u64 },
    /// `dom::on_submit(<var>, <handler>);` — form submit handler.
    OnSubmit { var: String, handler: u64 },
    /// `dom::on_change(<var>, <handler>);` — field change handler.
    OnChange { var: String, handler: u64 },
    /// `dom::on_input(<var>, <handler>);` — live input handler.
    OnInput { var: String, handler: u64 },
    /// `dom::on_play(<var>, <handler>);` — media playback start (P4).
    OnPlay { var: String, handler: u64 },
    /// `dom::on_pause(<var>, <handler>);` — media playback pause (P4).
    OnPause { var: String, handler: u64 },
    /// `dom::on_ended(<var>, <handler>);` — media playback ended (P4).
    OnEnded { var: String, handler: u64 },
}

/// A single field within a detected [`FormModel`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormField {
    /// Variable the field's element is bound to in the generated guest.
    pub var: String,
    /// `type` attribute (e.g. `text`, `number`, `email`); defaults to `text`.
    pub input_type: String,
    /// `name` attribute, used as the CRUD data-binding key (if present).
    pub name: Option<String>,
}

/// A form detected in the transpiled document (the "C" in CRUD).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormModel {
    /// Variable the `<form>` element is bound to.
    pub var: String,
    /// Fields making up the form.
    pub fields: Vec<FormField>,
    /// Whether the form carries a submit affordance (submit button / onsubmit).
    pub has_submit: bool,
}

/// A table detected in the transpiled document (the "R" in CRUD — the entity
/// listing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableModel {
    /// Variable the `<table>` element is bound to.
    pub var: String,
    /// Number of `<tr>` rows (data rows, including header rows).
    pub row_count: usize,
}

/// CRUD structure extracted from a P2 form/dashboard document.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CrudModel {
    /// Forms detected in document order.
    pub forms: Vec<FormModel>,
    /// Tables (entity listings) detected in document order.
    pub tables: Vec<TableModel>,
}

impl CrudModel {
    /// Every (form var, field var) binding discovered.
    pub fn field_bindings(&self) -> Vec<(String, String)> {
        self.forms
            .iter()
            .flat_map(|f| f.fields.iter().map(|field| (f.var.clone(), field.var.clone())))
            .collect()
    }

    /// Whether any form/table/handler semantics were detected.
    pub fn is_empty(&self) -> bool {
        self.forms.is_empty() && self.tables.is_empty()
    }
}

/// A chart/canvas element detected in a P3 data-visualization document (spec
/// §6.2 P3 — charts, real-time metric dashboards).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChartModel {
    /// Variable the `<canvas>`/`<svg>` element is bound to in the generated guest.
    pub var: String,
    /// Tag the chart is drawn in (`canvas` | `svg`).
    pub tag: String,
    /// `id` attribute (used as the chart handle / mount point).
    pub id: Option<String>,
    /// Pixel `width` (canvas) — `None` when absent or non-numeric.
    pub width: Option<usize>,
    /// Pixel `height` (canvas) — `None` when absent or non-numeric.
    pub height: Option<usize>,
    /// `data-*` series bindings (name → value), e.g. `data-series-0="revenue"`.
    pub series: Vec<(String, String)>,
}

/// Data-visualization structure extracted from a P3 document.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DataVizModel {
    /// Charts detected in document order.
    pub charts: Vec<ChartModel>,
}

impl DataVizModel {
    /// Whether any chart/canvas semantics were detected.
    pub fn is_empty(&self) -> bool {
        self.charts.is_empty()
    }
}

/// A `<source>` child of a [`MediaPlayerModel`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaSource {
    /// MIME type, e.g. `video/mp4` (from the `type` attribute).
    pub media_type: Option<String>,
    /// Resolved `src` for this alternate stream.
    pub src: String,
}

/// A media element detected in a P4 media-player document (spec §6.2 P4 —
/// video streaming, audio playback).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaPlayerModel {
    /// Variable the `<video>`/`<audio>` element is bound to.
    pub var: String,
    /// Kind of media (`video` | `audio`).
    pub kind: String,
    /// Primary `src` attribute (when the element carries one directly).
    pub src: Option<String>,
    /// Whether the native `controls` UI is requested.
    pub has_controls: bool,
    /// `autoplay` hint.
    pub autoplay: bool,
    /// `loop` hint.
    pub loop_playback: bool,
    /// Alternate streams declared via `<source>` children.
    pub sources: Vec<MediaSource>,
}

/// Media-player structure extracted from a P4 document.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MediaModel {
    /// Players detected in document order.
    pub players: Vec<MediaPlayerModel>,
}

impl MediaModel {
    /// Whether any media-player semantics were detected.
    pub fn is_empty(&self) -> bool {
        self.players.is_empty()
    }
}

/// The full transpilation output for one document (static or form-based CRUD).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranspiledSite {
    /// Ordered DOM operations rebuilding the page.
    pub ops: Vec<DomOp>,
    /// Generated guest Rust source (componentizes against `helix-guest`).
    pub rust_source: String,
    /// Generated WIT `world` snippet the guest is built from.
    pub wit_world: String,
    /// CRUD structure extracted from the document (empty for static sites).
    pub crud: CrudModel,
    /// Data-visualization structure extracted from the document (P3; empty
    /// unless the document contains chart/canvas elements).
    pub dataviz: DataVizModel,
    /// Media-player structure extracted from the document (P4; empty unless the
    /// document contains `<video>`/`<audio>` elements).
    pub media: MediaModel,
}

const VOID_TAGS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

fn is_void(tag: &str) -> bool {
    VOID_TAGS.contains(&tag)
}

/// Parse a static HTML document into a node forest (top-level nodes).
///
/// Tolerant of comments and doctype declarations (skipped). Whitespace-only
/// text runs are discarded so equivalence checks operate on meaningful content.
pub fn parse_html(input: &str) -> Vec<Node> {
    let chars: Vec<char> = input.chars().collect();
    let (nodes, _) = parse_nodes(&chars, 0, None);
    nodes
}

fn parse_nodes(chars: &[char], mut pos: usize, stop: Option<&str>) -> (Vec<Node>, usize) {
    let mut nodes = Vec::new();
    while pos < chars.len() {
        if chars[pos] == '<' {
            // Comment / doctype / declaration.
            if pos + 1 < chars.len() && (chars[pos + 1] == '!' || chars[pos + 1] == '?') {
                if pos + 3 < chars.len()
                    && chars[pos + 1..pos + 4] == ['!', '-', '-']
                    && let Some(end) = find_substr(chars, pos + 4, "-->")
                {
                    pos = end;
                    continue;
                }
                if let Some(end) = find_char(chars, pos + 1, '>') {
                    pos = end + 1;
                    continue;
                }
                pos += 1;
                continue;
            }
            // Closing tag.
            if pos + 1 < chars.len() && chars[pos + 1] == '/' {
                let (name, end) = read_tag_name(chars, pos + 2);
                // `end` points at '>'; advance past it so the caller does not
                // treat the '>' as a stray text node.
                if let Some(stop) = stop
                    && name.eq_ignore_ascii_case(stop)
                {
                    return (nodes, end + 1);
                }
                pos = end + 1;
                continue;
            }
            // Opening tag.
            let (el, end, self_close) = parse_element(chars, pos);
            pos = end;
            if self_close || is_void(&el.tag) {
                nodes.push(Node::Element(el));
                continue;
            }
            let (children, after) = parse_nodes(chars, pos, Some(&el.tag));
            pos = after;
            let mut el = el;
            el.children = children;
            nodes.push(Node::Element(el));
        } else {
            let (text, end) = read_text(chars, pos);
            pos = end;
            let trimmed: String = text.split_whitespace().collect();
            if !trimmed.is_empty() {
                nodes.push(Node::Text(trimmed));
            }
        }
    }
    (nodes, pos)
}

fn parse_element(chars: &[char], start: usize) -> (Element, usize, bool) {
    let (tag, mut pos) = read_tag_name(chars, start + 1);
    let mut attrs = Vec::new();
    let mut self_close = false;
    while pos < chars.len() {
        match chars[pos] {
            '>' => {
                pos += 1;
                break;
            }
            '/' => {
                self_close = true;
                if pos + 1 < chars.len() && chars[pos + 1] == '>' {
                    pos += 2;
                    break;
                }
                pos += 1;
            }
            c if c.is_whitespace() => pos += 1,
            _ => {
                let (name, val, end) = read_attr(chars, pos);
                pos = end;
                attrs.push((name, val));
            }
        }
    }
    (
        Element {
            tag,
            attrs,
            children: Vec::new(),
        },
        pos,
        self_close,
    )
}

fn read_tag_name(chars: &[char], start: usize) -> (String, usize) {
    let mut pos = start;
    while pos < chars.len() && chars[pos].is_whitespace() {
        pos += 1;
    }
    let begin = pos;
    while pos < chars.len() && !chars[pos].is_whitespace() && chars[pos] != '>' && chars[pos] != '/'
    {
        pos += 1;
    }
    (chars[begin..pos].iter().collect(), pos)
}

fn read_attr(chars: &[char], start: usize) -> (String, String, usize) {
    let begin = start;
    let mut pos = start;
    while pos < chars.len()
        && !chars[pos].is_whitespace()
        && chars[pos] != '='
        && chars[pos] != '>'
        && chars[pos] != '/'
    {
        pos += 1;
    }
    let name: String = chars[begin..pos].iter().collect();
    // Skip whitespace before '='.
    while pos < chars.len() && chars[pos].is_whitespace() {
        pos += 1;
    }
    let mut value = String::new();
    if pos < chars.len() && chars[pos] == '=' {
        pos += 1;
        while pos < chars.len() && chars[pos].is_whitespace() {
            pos += 1;
        }
        if pos < chars.len() && (chars[pos] == '"' || chars[pos] == '\'') {
            let q = chars[pos];
            pos += 1;
            let vbegin = pos;
            while pos < chars.len() && chars[pos] != q {
                pos += 1;
            }
            value = chars[vbegin..pos].iter().collect();
            if pos < chars.len() {
                pos += 1; // closing quote
            }
        } else {
            // Unquoted value.
            let vbegin = pos;
            while pos < chars.len()
                && !chars[pos].is_whitespace()
                && chars[pos] != '>'
                && chars[pos] != '/'
            {
                pos += 1;
            }
            value = chars[vbegin..pos].iter().collect();
        }
    }
    (name, value, pos)
}

fn read_text(chars: &[char], start: usize) -> (String, usize) {
    let begin = start;
    let mut pos = start;
    while pos < chars.len() && chars[pos] != '<' {
        pos += 1;
    }
    (chars[begin..pos].iter().collect(), pos)
}

fn find_char(chars: &[char], start: usize, target: char) -> Option<usize> {
    (start..chars.len()).find(|&i| chars[i] == target)
}

fn find_substr(chars: &[char], start: usize, pat: &str) -> Option<usize> {
    let pat: Vec<char> = pat.chars().collect();
    if pat.is_empty() {
        return Some(start);
    }
    (start..chars.len())
        .find(|&i| chars[i..].starts_with(&pat))
        .map(|i| i + pat.len())
}

/// Count element and text nodes in a parsed forest (whitespace text excluded).
pub fn count_nodes(nodes: &[Node]) -> (usize, usize) {
    let mut elements = 0;
    let mut texts = 0;
    fn rec(nodes: &[Node], e: &mut usize, t: &mut usize) {
        for n in nodes {
            match n {
                Node::Element(el) => {
                    *e += 1;
                    rec(&el.children, e, t);
                }
                Node::Text(_) => *t += 1,
            }
        }
    }
    rec(nodes, &mut elements, &mut texts);
    (elements, texts)
}

/// Collect text runs in document order.
pub fn collect_text(nodes: &[Node]) -> Vec<String> {
    let mut out = Vec::new();
    fn rec(nodes: &[Node], out: &mut Vec<String>) {
        for n in nodes {
            match n {
                Node::Element(el) => rec(&el.children, out),
                Node::Text(s) => out.push(s.clone()),
            }
        }
    }
    rec(nodes, &mut out);
    out
}

fn rust_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Event-handler attributes mapped to their [`DomOp`] variant. These are
/// captured as handler operations rather than raw `set-attribute` calls so the
/// generated guest wires interactivity explicitly.
fn handler_attr(name: &str) -> Option<fn(String, u64) -> DomOp> {
    match name.to_ascii_lowercase().as_str() {
        "onclick" => Some(|v, h| DomOp::OnClick {
            var: v,
            handler: h,
        }),
        "onsubmit" => Some(|v, h| DomOp::OnSubmit {
            var: v,
            handler: h,
        }),
        "onchange" => Some(|v, h| DomOp::OnChange {
            var: v,
            handler: h,
        }),
        "oninput" => Some(|v, h| DomOp::OnInput {
            var: v,
            handler: h,
        }),
        "onplay" => Some(|v, h| DomOp::OnPlay {
            var: v,
            handler: h,
        }),
        "onpause" => Some(|v, h| DomOp::OnPause {
            var: v,
            handler: h,
        }),
        "onended" => Some(|v, h| DomOp::OnEnded {
            var: v,
            handler: h,
        }),
        _ => None,
    }
}

struct Emitter {
    ops: Vec<DomOp>,
    counter: usize,
    handler_counter: u64,
    crud: CrudModel,
    dataviz: DataVizModel,
    media: MediaModel,
}

impl Emitter {
    fn next_var(&mut self) -> String {
        let v = format!("el{}", self.counter);
        self.counter += 1;
        v
    }

    fn next_handler(&mut self) -> u64 {
        let h = self.handler_counter;
        self.handler_counter += 1;
        h
    }

    fn emit(
        &mut self,
        nodes: &[Node],
        parent: Option<&str>,
        cur_form: Option<usize>,
        cur_table: Option<usize>,
    ) {
        for node in nodes {
            match node {
                Node::Element(el) => {
                    let var = self.next_var();
                    self.ops.push(DomOp::Create {
                        var: var.clone(),
                        tag: el.tag.clone(),
                    });

                    // Track form/table context so fields/rows bind to the right
                    // container as we recurse.
                    let mut this_form = cur_form;
                    if el.tag.eq_ignore_ascii_case("form") {
                        let idx = self.crud.forms.len();
                        self.crud.forms.push(FormModel {
                            var: var.clone(),
                            fields: Vec::new(),
                            has_submit: false,
                        });
                        this_form = Some(idx);
                    }
                    let mut this_table = cur_table;
                    if el.tag.eq_ignore_ascii_case("table") {
                        let idx = self.crud.tables.len();
                        self.crud.tables.push(TableModel {
                            var: var.clone(),
                            row_count: 0,
                        });
                        this_table = Some(idx);
                    }
                    if el.tag.eq_ignore_ascii_case("tr") {
                        if let Some(t) = this_table {
                            self.crud.tables[t].row_count += 1;
                        }
                    }

                    // P3 data-visualization: a `<canvas>`/`<svg>` chart mount
                    // point. Record id/width/height and any `data-*` series.
                    if el.tag.eq_ignore_ascii_case("canvas") || el.tag.eq_ignore_ascii_case("svg") {
                        let id = el
                            .attrs
                            .iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case("id"))
                            .map(|(_, v)| v.clone());
                        let width = el
                            .attrs
                            .iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case("width"))
                            .and_then(|(_, v)| v.parse::<usize>().ok());
                        let height = el
                            .attrs
                            .iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case("height"))
                            .and_then(|(_, v)| v.parse::<usize>().ok());
                        let series = el
                            .attrs
                            .iter()
                            .filter(|(k, _)| k.to_ascii_lowercase().starts_with("data-"))
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect();
                        self.dataviz.charts.push(ChartModel {
                            var: var.clone(),
                            tag: el.tag.clone(),
                            id,
                            width,
                            height,
                            series,
                        });
                    }

                    // P4 media player: a `<video>`/`<audio>` element with its
                    // primary `src`, UI/playback hints, and any `<source>` kids.
                    if el.tag.eq_ignore_ascii_case("video") || el.tag.eq_ignore_ascii_case("audio") {
                        let src = el
                            .attrs
                            .iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case("src"))
                            .map(|(_, v)| v.clone());
                        let has_controls = el
                            .attrs
                            .iter()
                            .any(|(k, _)| k.eq_ignore_ascii_case("controls"));
                        let autoplay = el
                            .attrs
                            .iter()
                            .any(|(k, _)| k.eq_ignore_ascii_case("autoplay"));
                        let loop_playback = el
                            .attrs
                            .iter()
                            .any(|(k, _)| k.eq_ignore_ascii_case("loop"));
                        let sources = el
                            .children
                            .iter()
                            .filter_map(|c| match c {
                                Node::Element(child)
                                    if child.tag.eq_ignore_ascii_case("source") =>
                                {
                                    let s = child
                                        .attrs
                                        .iter()
                                        .find(|(k, _)| k.eq_ignore_ascii_case("src"))
                                        .map(|(_, v)| v.clone());
                                    let t = child
                                        .attrs
                                        .iter()
                                        .find(|(k, _)| k.eq_ignore_ascii_case("type"))
                                        .map(|(_, v)| v.clone());
                                    s.map(|src| MediaSource {
                                        media_type: t,
                                        src,
                                    })
                                }
                                _ => None,
                            })
                            .collect();
                        self.media.players.push(MediaPlayerModel {
                            var: var.clone(),
                            kind: el.tag.clone(),
                            src,
                            has_controls,
                            autoplay,
                            loop_playback,
                            sources,
                        });
                    }


                    // Attributes: handler attributes become handler ops; the rest
                    // are plain `set-attribute` calls. Field `name`/`type` are
                    // recorded for the CRUD data-binding model.
                    let mut field_name: Option<String> = None;
                    let mut field_type: Option<String> = None;
                    let is_field = el.tag.eq_ignore_ascii_case("input")
                        || el.tag.eq_ignore_ascii_case("select")
                        || el.tag.eq_ignore_ascii_case("textarea");
                    for (name, value) in &el.attrs {
                        if let Some(make) = handler_attr(name) {
                            let h = self.next_handler();
                            self.ops.push(make(var.clone(), h));
                            if el.tag.eq_ignore_ascii_case("form")
                                && name.eq_ignore_ascii_case("onsubmit")
                            {
                                if let Some(f) = this_form {
                                    self.crud.forms[f].has_submit = true;
                                }
                            }
                            continue;
                        }
                        if is_field {
                            if name.eq_ignore_ascii_case("name") {
                                field_name = Some(value.clone());
                            } else if name.eq_ignore_ascii_case("type") {
                                field_type = Some(value.clone());
                            }
                        }
                        self.ops.push(DomOp::Attr {
                            var: var.clone(),
                            name: name.clone(),
                            value: value.clone(),
                        });
                    }
                    if is_field {
                        if let Some(f) = this_form {
                            self.crud.forms[f].fields.push(FormField {
                                var: var.clone(),
                                input_type: field_type.unwrap_or_else(|| "text".to_string()),
                                name: field_name,
                            });
                        }
                    }
                    if el.tag.eq_ignore_ascii_case("button") {
                        // A submit button inside a form confers submit capability
                        // even when no explicit `onsubmit` handler is declared.
                        if let Some(f) = this_form {
                            let is_submit = el
                                .attrs
                                .iter()
                                .any(|(k, v)| k.eq_ignore_ascii_case("type") && v.eq_ignore_ascii_case("submit"));
                            if is_submit {
                                self.crud.forms[f].has_submit = true;
                            }
                        }
                    }

                    // Text handling: pure-text leaf -> set-text on element;
                    // mixed content -> span-wrapped text children.
                    let text_runs: Vec<&String> = el
                        .children
                        .iter()
                        .filter_map(|c| match c {
                            Node::Text(t) => Some(t),
                            _ => None,
                        })
                        .collect();
                    let has_elements = el.children.iter().any(|c| matches!(c, Node::Element(_)));
                    if text_runs.len() == 1 && !has_elements {
                        self.ops.push(DomOp::Text {
                            var: var.clone(),
                            text: text_runs[0].clone(),
                        });
                    } else {
                        for t in text_runs {
                            let span = self.next_var();
                            self.ops.push(DomOp::Create {
                                var: span.clone(),
                                tag: "span".to_string(),
                            });
                            self.ops.push(DomOp::Text {
                                var: span.clone(),
                                text: t.clone(),
                            });
                            self.ops.push(DomOp::Append {
                                parent: var.clone(),
                                child: span,
                            });
                        }
                    }
                    // Element children.
                    let child_elements: Vec<Node> = el
                        .children
                        .iter()
                        .filter(|c| matches!(c, Node::Element(_)))
                        .cloned()
                        .collect();
                    self.emit(&child_elements, Some(&var), this_form, this_table);
                    if let Some(p) = parent {
                        self.ops.push(DomOp::Append {
                            parent: p.to_string(),
                            child: var,
                        });
                    }
                }
                Node::Text(_) => {
                    // Top-level text (no parent) — wrap in a span root.
                    let span = self.next_var();
                    self.ops.push(DomOp::Create {
                        var: span.clone(),
                        tag: "span".to_string(),
                    });
                    if let Node::Text(t) = node {
                        self.ops.push(DomOp::Text {
                            var: span.clone(),
                            text: t.clone(),
                        });
                    }
                }
            }
        }
    }

    fn generate_rust(&self) -> String {
        let mut s = String::new();
        s.push_str("wit_bindgen::generate!({\n    world: \"helix-guest\",\n    path: \"../helix-wit/wit\",\n});\n\n");
        s.push_str(
            "#[unsafe(no_mangle)]\npub extern \"C\" fn run() {\n    use helix::runtime::dom;\n",
        );
        for op in &self.ops {
            match op {
                DomOp::Create { var, tag } => {
                    let _ = writeln!(
                        s,
                        "    let {var} = dom::create_element(&\"{}\");",
                        rust_escape(tag)
                    );
                }
                DomOp::Text { var, text } => {
                    let _ = writeln!(s, "    dom::set_text({var}, &\"{}\");", rust_escape(text));
                }
                DomOp::Attr { var, name, value } => {
                    let _ = writeln!(
                        s,
                        "    dom::set_attribute({var}, &\"{}\", &\"{}\");",
                        rust_escape(name),
                        rust_escape(value)
                    );
                }
                DomOp::Append { parent, child } => {
                    let _ = writeln!(s, "    dom::append_child({parent}, {child});");
                }
                DomOp::OnClick { var, handler } => {
                    let _ = writeln!(s, "    dom::on_click({var}, {handler});");
                }
                DomOp::OnSubmit { var, handler } => {
                    let _ = writeln!(s, "    dom::on_submit({var}, {handler});");
                }
                DomOp::OnChange { var, handler } => {
                    let _ = writeln!(s, "    dom::on_change({var}, {handler});");
                }
                DomOp::OnInput { var, handler } => {
                    let _ = writeln!(s, "    dom::on_input({var}, {handler});");
                }
                DomOp::OnPlay { var, handler } => {
                    let _ = writeln!(s, "    dom::on_play({var}, {handler});");
                }
                DomOp::OnPause { var, handler } => {
                    let _ = writeln!(s, "    dom::on_pause({var}, {handler});");
                }
                DomOp::OnEnded { var, handler } => {
                    let _ = writeln!(s, "    dom::on_ended({var}, {handler});");
                }
            }
        }
        s.push_str("}\n");
        s
    }
}

/// Transpile an HTML document (static or form-based CRUD) into a Helix guest
/// component, capturing any interactive/CRUD structure it may contain.
fn transpile_html(html: &str) -> TranspiledSite {
    let nodes = parse_html(html);
    let mut emitter = Emitter {
        ops: Vec::new(),
        counter: 0,
        handler_counter: 0,
        crud: CrudModel::default(),
        dataviz: DataVizModel::default(),
        media: MediaModel::default(),
    };
    emitter.emit(&nodes, None, None, None);
    let ops = emitter.ops.clone();
    let crud = emitter.crud.clone();
    let dataviz = emitter.dataviz.clone();
    let media = emitter.media.clone();
    let rust_source = emitter.generate_rust();
    let wit_world = r#"world helix-guest {
    import dom;
    export run: func();
}
"#
    .to_string();
    TranspiledSite {
        ops,
        rust_source,
        wit_world,
        crud,
        dataviz,
        media,
    }
}

/// Transpile a static HTML document into a Helix guest component.
///
/// See [`transpile_form_app`] for the P2 form-based CRUD variant; both share the
/// same underlying pipeline. Static documents yield an empty [`CrudModel`].
pub fn transpile_static_site(html: &str) -> TranspiledSite {
    transpile_html(html)
}

/// Transpile a P2 form-based CRUD / dashboard document into a Helix guest.
///
/// Beyond the static content model, this captures form fields, submit/change/
/// input/click handlers, and tabular entity listings so Stage S3 can assert
/// CRUD equivalence (fields bind to a form, submit is wired, rows map 1:1).
pub fn transpile_form_app(html: &str) -> TranspiledSite {
    transpile_html(html)
}

/// Transpile a P3 data-visualization document (charts, real-time metric
/// dashboards) into a Helix guest.
///
/// Extends the shared pipeline with [`DataVizModel`] extraction: every
/// `<canvas>`/`<svg>` chart mount is recorded with its `id`, pixel `width`/
/// `height`, and any `data-*` series bindings, so Stage S3 can assert chart
/// equivalence (mount point present, dimensions resolved, series bound).
pub fn transpile_data_viz(html: &str) -> TranspiledSite {
    transpile_html(html)
}

/// Transpile a P4 media-player document (video streaming, audio playback) into
/// a Helix guest.
///
/// Extends the shared pipeline with [`MediaModel`] extraction: every
/// `<video>`/`<audio>` element records its `src`, `controls`/`autoplay`/`loop`
/// hints, and any `<source>` alternate streams, plus `onplay`/`onpause`/`onended`
/// handler wiring, so Stage S3 can assert media equivalence (player present,
/// source resolved, controls requested, sources enumerated).
pub fn transpile_media_player(html: &str) -> TranspiledSite {
    transpile_html(html)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_elements_and_attributes() {
        let html = r#"<html><head><title>T</title></head><body class="x"><p>Hi <b>there</b></p></body></html>"#;
        let nodes = parse_html(html);
        let (elements, texts) = count_nodes(&nodes);
        assert_eq!(elements, 6); // html, head, title, body, p, b
        assert_eq!(texts, 3); // T, Hi, there
        let text = collect_text(&nodes);
        assert_eq!(text, vec!["T", "Hi", "there"]);
    }

    #[test]
    fn skips_comments_and_doctype_and_whitespace() {
        let html = r#"<!doctype html><!-- ignore --><div>   </div><p>real</p>"#;
        let nodes = parse_html(html);
        let (elements, texts) = count_nodes(&nodes);
        assert_eq!(elements, 2); // div, p
        assert_eq!(texts, 1); // real
    }

    #[test]
    fn transpile_emits_ops_matching_source_shape() {
        let html = r#"<body><h1>Title</h1><p>Hello <b>world</b></p></body>"#;
        let site = transpile_static_site(html);
        let (elements, texts) = count_nodes(&parse_html(html));
        // Each element yields one Create; mixed-content text runs add a span.
        let creates = site
            .ops
            .iter()
            .filter(|o| matches!(o, DomOp::Create { .. }))
            .count();
        let set_texts = site
            .ops
            .iter()
            .filter(|o| matches!(o, DomOp::Text { .. }))
            .count();
        assert!(creates >= elements, "every element must be created");
        // Every text run maps to exactly one set_text (on element or span).
        assert_eq!(set_texts, texts, "text runs must be preserved 1:1");
        // Every Append has a defined parent (parent index < child index).
        let mut created = std::collections::HashSet::new();
        for op in &site.ops {
            if let DomOp::Create { var, .. } = op {
                created.insert(var.clone());
            }
            if let DomOp::Append { parent, child } = op {
                assert!(
                    created.contains(parent),
                    "append references undefined parent"
                );
                assert!(created.contains(child), "append references undefined child");
            }
        }
    }

    #[test]
    fn generated_rust_is_well_formed() {
        let site = transpile_static_site(r#"<div class="a"><span>hi</span></div>"#);
        assert!(site.rust_source.contains("wit_bindgen::generate!"));
        assert!(site.rust_source.contains("pub extern \"C\" fn run()"));
        assert!(site.rust_source.contains("dom::create_element(&\"div\")"));
        assert!(
            site.rust_source
                .contains("dom::set_attribute(el0, &\"class\", &\"a\")")
        );
        assert!(site.rust_source.contains("dom::append_child(el0, el1)"));
        assert!(site.wit_world.contains("export run: func()"));
        // Static sites carry no interactive/CRUD structure.
        assert!(site.crud.is_empty());
    }

    #[test]
    fn form_app_captures_fields_and_submit_handler() {
        let html = r#"
            <form onsubmit="add()">
                <label>Name</label>
                <input type="text" name="name" />
                <input type="number" name="age" />
                <button type="submit">Add</button>
            </form>"#;
        let site = transpile_form_app(html);

        // One form, carrying two fields and a submit affordance.
        assert_eq!(site.crud.forms.len(), 1);
        let form = &site.crud.forms[0];
        assert_eq!(form.fields.len(), 2);
        assert!(form.has_submit);
        assert_eq!(form.fields[0].name.as_deref(), Some("name"));
        assert_eq!(form.fields[0].input_type, "text");
        assert_eq!(form.fields[1].name.as_deref(), Some("age"));
        assert_eq!(form.fields[1].input_type, "number");

        // The form must carry an `onsubmit` handler op (not a raw attribute).
        let has_submit_handler = site.ops.iter().any(|o| {
            matches!(o, DomOp::OnSubmit { var, .. } if var == &form.var)
        });
        assert!(has_submit_handler, "form must emit a submit handler");

        // Field `name`/`type` must be present as attributes for data-binding.
        let attr_names: Vec<&str> = site
            .ops
            .iter()
            .filter_map(|o| match o {
                DomOp::Attr { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert!(attr_names.contains(&"name"));
        assert!(attr_names.contains(&"type"));

        // The label text is preserved exactly.
        let text: Vec<String> = site
            .ops
            .iter()
            .filter_map(|o| match o {
                DomOp::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert!(text.contains(&"Name".to_string()));
    }

    #[test]
    fn form_app_captures_table_rows_as_entity_listing() {
        let html = r#"
            <table>
                <tr><td>a</td></tr>
                <tr><td>b</td></tr>
                <tr><td>c</td></tr>
            </table>"#;
        let site = transpile_form_app(html);

        assert_eq!(site.crud.tables.len(), 1);
        assert_eq!(site.crud.tables[0].row_count, 3);

        // Forward-reference soundness: every append resolves.
        let mut created = std::collections::HashSet::new();
        for op in &site.ops {
            if let DomOp::Create { var, .. } = op {
                created.insert(var.clone());
            }
            if let DomOp::Append { parent, child } = op {
                assert!(created.contains(parent), "append to undefined {parent}");
                assert!(created.contains(child), "append of undefined {child}");
            }
        }
    }

    #[test]
    fn form_app_generates_handler_wiring_in_rust() {
        let html = r#"<form onsubmit="save()"><input name="x" onchange="upd()" /></form>"#;
        let site = transpile_form_app(html);
        assert!(site.rust_source.contains("dom::on_submit("));
        assert!(site.rust_source.contains("dom::on_change("));
        // Handlers must not leak as raw attributes.
        assert!(!site.rust_source.contains("set_attribute(el0, &\"onsubmit\""));
    }

    #[test]
    fn data_viz_captures_chart_mount_and_series() {
        let html = r#"<h1>Dashboard</h1><canvas id="chart" width="800" height="600" data-series-0="revenue" data-series-1="cost"></canvas>"#;
        let site = transpile_data_viz(html);

        // One chart mount captured with resolved dimensions and series.
        assert_eq!(site.dataviz.charts.len(), 1);
        let chart = &site.dataviz.charts[0];
        assert_eq!(chart.tag, "canvas");
        assert_eq!(chart.id.as_deref(), Some("chart"));
        assert_eq!(chart.width, Some(800));
        assert_eq!(chart.height, Some(600));
        assert_eq!(chart.series.len(), 2);
        assert_eq!(chart.series[0], ("data-series-0".to_string(), "revenue".to_string()));
        assert_eq!(chart.series[1], ("data-series-1".to_string(), "cost".to_string()));

        // The chart's dimensions/series must surface as attributes on the
        // created element (so the guest can reconstruct it).
        let attrs: Vec<(String, String)> = site
            .ops
            .iter()
            .filter_map(|o| match o {
                DomOp::Attr { var, name, value } if var == &chart.var => {
                    Some((name.clone(), value.clone()))
                }
                _ => None,
            })
            .collect();
        assert!(attrs.iter().any(|(k, v)| k == "id" && v == "chart"));
        assert!(attrs.iter().any(|(k, v)| k == "width" && v == "800"));
        assert!(attrs.iter().any(|(k, v)| k == "height" && v == "600"));
        assert!(attrs.iter().any(|(k, v)| k == "data-series-0" && v == "revenue"));

        // Heading text preserved exactly.
        let text: Vec<String> = site
            .ops
            .iter()
            .filter_map(|o| match o {
                DomOp::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert!(text.contains(&"Dashboard".to_string()));

        // Append soundness.
        let mut created = std::collections::HashSet::new();
        for op in &site.ops {
            if let DomOp::Create { var, .. } = op {
                created.insert(var.clone());
            }
            if let DomOp::Append { parent, child } = op {
                assert!(created.contains(parent) && created.contains(child));
            }
        }
    }

    #[test]
    fn data_viz_also_detects_svg_charts() {
        let html = r#"<svg id="gauge" width="200" height="200"></svg>"#;
        let site = transpile_data_viz(html);
        assert_eq!(site.dataviz.charts.len(), 1);
        assert_eq!(site.dataviz.charts[0].tag, "svg");
        assert_eq!(site.dataviz.charts[0].id.as_deref(), Some("gauge"));
    }

    #[test]
    fn media_player_captures_src_controls_and_sources() {
        let html = r#"
            <h1>Player</h1>
            <video src="main.mp4" controls>
                <source src="main.webm" type="video/webm"></source>
                <source src="main.ogg" type="video/ogg"></source>
            </video>"#;
        let site = transpile_media_player(html);

        assert_eq!(site.media.players.len(), 1);
        let player = &site.media.players[0];
        assert_eq!(player.kind, "video");
        assert_eq!(player.src.as_deref(), Some("main.mp4"));
        assert!(player.has_controls);
        assert!(!player.autoplay);
        assert!(!player.loop_playback);
        assert_eq!(player.sources.len(), 2);
        assert_eq!(player.sources[0].src, "main.webm");
        assert_eq!(player.sources[0].media_type.as_deref(), Some("video/webm"));
        assert_eq!(player.sources[1].src, "main.ogg");

        // Heading text preserved exactly.
        let text: Vec<String> = site
            .ops
            .iter()
            .filter_map(|o| match o {
                DomOp::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert!(text.contains(&"Player".to_string()));

        // Source children must not be emitted as top-level elements (they are
        // folded into the player model).
        let creates: Vec<String> = site
            .ops
            .iter()
            .filter_map(|o| match o {
                DomOp::Create { tag, .. } => Some(tag.clone()),
                _ => None,
            })
            .collect();
        assert!(!creates.contains(&"source".to_string()));

        // Append soundness.
        let mut created = std::collections::HashSet::new();
        for op in &site.ops {
            if let DomOp::Create { var, .. } = op {
                created.insert(var.clone());
            }
            if let DomOp::Append { parent, child } = op {
                assert!(created.contains(parent) && created.contains(child));
            }
        }
    }

    #[test]
    fn media_player_wires_playback_event_handlers() {
        let html = r#"<audio src="podcast.mp3" controls onplay="started()" onpause="paused()" onended="done()"></audio>"#;
        let site = transpile_media_player(html);
        assert!(site.rust_source.contains("dom::on_play("));
        assert!(site.rust_source.contains("dom::on_pause("));
        assert!(site.rust_source.contains("dom::on_ended("));
        // Playback handlers must not leak as raw attributes.
        assert!(!site.rust_source.contains("set_attribute(el0, &\"onplay\""));
        assert_eq!(site.media.players[0].kind, "audio");
    }
}
