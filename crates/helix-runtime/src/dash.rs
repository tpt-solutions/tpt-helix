//! DASH adaptive streaming client (spec §4 / Phase 1 "Media pipeline").
//!
//! This module is the *client-side* half of DASH: parse an MPD manifest, model
//! its AdaptationSets / Representations / segments, and run an adaptive bitrate
//! (ABR) selector that picks the best Representation for a given bandwidth
//! budget. Segment URL construction (SegmentTemplate / SegmentList / BaseURL)
//! is fully implemented so a host can drive `media` capability playback.
//!
//! The decode half (turning a fetched segment's bytes into frames) is the
//! `DecoderBackend` in `media_decode.rs`; the two are deliberately independent
//! so ABR can be unit-tested without a GPU or codec.

use std::collections::HashMap;

/// Errors produced while parsing an MPD manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DashError {
    /// A required element/attribute was missing or malformed.
    Malformed(String),
}

/// A single media segment as addressed by the manifest.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// Absolute (post-template-substitution) URL of the segment.
    pub url: String,
    /// Zero-based segment index (from `$Number$`, when templated).
    pub number: u64,
    /// Segment duration in seconds (from the template `duration`/`timescale`).
    pub duration_secs: f64,
}

/// `SegmentTemplate` parameters used to synthesize segment URLs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SegmentTemplate {
    /// URL template for media segments (`$RepresentationID$`, `$Number$`,
    /// `$Bandwidth$`, `$Time$` substitutions).
    pub media: String,
    /// URL template for the initialization segment.
    pub initialization: String,
    /// Ticks per second for `$Time$` substitution and duration math.
    pub timescale: u64,
    /// First segment number (defaults to 1).
    pub start_number: u64,
    /// Segment duration in `timescale` ticks.
    pub duration: u64,
}

/// A playable representation (one bitrate/codec/resolution ladder rung).
#[derive(Debug, Clone, PartialEq)]
pub struct Representation {
    pub id: String,
    pub bandwidth: u64,
    pub codecs: String,
    pub mime_type: String,
    pub width: u32,
    pub height: u32,
    /// Base URL inherited from ancestor elements (`BaseURL`).
    pub base_url: String,
    pub segment_template: Option<SegmentTemplate>,
    /// Explicit segment list (SegmentList), if the manifest does not template.
    pub segments: Vec<Segment>,
}

impl Representation {
    /// The resolution as an `(width, height)` pair (0,0 if unspecified).
    pub fn resolution(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Duration of one templated segment in seconds (0.0 if not templated).
    pub fn segment_duration_secs(&self) -> f64 {
        match &self.segment_template {
            Some(t) if t.timescale > 0 => t.duration as f64 / t.timescale as f64,
            _ => 0.0,
        }
    }

    /// URL of the initialization segment, if templated.
    pub fn initialization_url(&self) -> Option<String> {
        let t = self.segment_template.as_ref()?;
        if t.initialization.is_empty() {
            return None;
        }
        Some(substitute(
            &t.initialization,
            &self.id,
            self.bandwidth,
            t.start_number,
            0,
        ))
    }

    /// Synthesize the segment URLs for the templated portion of this
    /// representation, up to `count` segments starting at `start_number`.
    pub fn templated_segments(&self, count: u64) -> Vec<Segment> {
        let Some(t) = &self.segment_template else {
            return Vec::new();
        };
        let dur = self.segment_duration_secs();
        let base = if self.base_url.is_empty() {
            String::new()
        } else {
            ensure_trailing_slash(&self.base_url)
        };
        let start = if t.start_number == 0 { 1 } else { t.start_number };
        (0..count)
            .map(|i| {
                let n = start + i;
                let time = (n.saturating_sub(start)) * t.duration;
                Segment {
                    url: format!(
                        "{}{}",
                        base,
                        substitute(&t.media, &self.id, self.bandwidth, n, time)
                    ),
                    number: n,
                    duration_secs: dur,
                }
            })
            .collect()
    }
}

/// A group of equivalent representations (e.g. the video ladder).
#[derive(Debug, Clone, PartialEq)]
pub struct AdaptationSet {
    pub content_type: String,
    pub representations: Vec<Representation>,
}

/// A parsed MPD manifest, in client-friendly form.
#[derive(Debug, Clone, PartialEq)]
pub struct Mpd {
    /// Media presentation duration in seconds.
    pub duration_secs: f64,
    /// Suggested client buffer (seconds).
    pub min_buffer_secs: f64,
    pub adaptation_sets: Vec<AdaptationSet>,
}

impl Mpd {
    /// All representations across all adaptation sets.
    pub fn representations(&self) -> impl Iterator<Item = &Representation> {
        self.adaptation_sets
            .iter()
            .flat_map(|a| a.representations.iter())
    }

    /// Representations of a given content type (e.g. `"video"`).
    pub fn by_type(&self, content_type: &str) -> Vec<&Representation> {
        self.adaptation_sets
            .iter()
            .filter(|a| a.content_type.eq_ignore_ascii_case(content_type))
            .flat_map(|a| a.representations.iter())
            .collect()
    }

    /// Total segment count for a representation given the presentation duration.
    pub fn segment_count(&self, rep: &Representation) -> u64 {
        let dur = rep.segment_duration_secs();
        if dur <= 0.0 || self.duration_secs <= 0.0 {
            return rep.segments.len() as u64;
        }
        (self.duration_secs / dur).ceil() as u64
    }
}

// --- parsing ---------------------------------------------------------------

/// Parse an MPD manifest string into a client model.
///
/// Tolerant of the common DASH shapes (SegmentTemplate, SegmentList, BaseURL
/// inheritance, attribute-quoted with `"` or `'`). Unknown elements are skipped.
pub fn parse_mpd(xml: &str) -> Result<Mpd, DashError> {
    let root = find_element(xml, "MPD")
        .ok_or_else(|| DashError::Malformed("no MPD root element".into()))?;
    let (mpd_attrs, mpd_inner) = root;

    let duration_secs = parse_duration_attr(mpd_attrs.get("mediaPresentationDuration"));
    let min_buffer_secs = mpd_attrs
        .get("minBufferTime")
        .map(|s| parse_duration_attr(Some(s)))
        .unwrap_or(0.0);

    let mut base_url = String::new();
    if let Some((attrs, inner)) = find_element(&mpd_inner, "BaseURL") {
        base_url = inner.trim().to_string();
        let _ = attrs;
    }

    let mut adaptation_sets = Vec::new();
    for (aset_attrs, aset_inner) in find_all(&mpd_inner, "AdaptationSet") {
        let content_type = aset_attrs
            .get("contentType")
            .cloned()
            .or_else(|| aset_attrs.get("mimeType").map(|m| mime_to_type(m)))
            .unwrap_or_default();
        let mut set_base = base_url.clone();
        if let Some((_, inner)) = find_element(&aset_inner, "BaseURL") {
            set_base = inner.trim().to_string();
        }
        let mut reps = Vec::new();
        for (rep_attrs, rep_inner) in find_all(&aset_inner, "Representation") {
            let id = rep_attrs
                .get("id")
                .cloned()
                .unwrap_or_else(|| "<no-id>".into());
            let bandwidth = rep_attrs
                .get("bandwidth")
                .and_then(|b| b.parse::<u64>().ok())
                .unwrap_or(0);
            let mut rep_base = set_base.clone();
            if let Some((_, inner)) = find_element(&rep_inner, "BaseURL") {
                rep_base = inner.trim().to_string();
            }
            let segment_template = find_element(&rep_inner, "SegmentTemplate")
                .or_else(|| find_element(&aset_inner, "SegmentTemplate"))
                .map(|(attrs, _)| parse_template(&attrs));
            let mut segments = Vec::new();
            if let Some((_, list_inner)) = find_element(&rep_inner, "SegmentList")
                .or_else(|| find_element(&aset_inner, "SegmentList"))
            {
                for (seg_attrs, seg_inner) in find_all(&list_inner, "SegmentURL") {
                    let url = seg_attrs
                        .get("media")
                        .cloned()
                        .or_else(|| {
                            find_element(&seg_inner, "BaseURL")
                                .map(|(_, i)| format!("{}{}", ensure_trailing_slash(&rep_base), i.trim()))
                        })
                        .unwrap_or_default();
                    let dur = seg_attrs
                        .get("d")
                        .and_then(|d| d.parse::<u64>().ok())
                        .map(|d| d as f64 / 1000.0)
                        .unwrap_or(0.0);
                    segments.push(Segment {
                        url,
                        number: segments.len() as u64,
                        duration_secs: dur,
                    });
                }
            }
            reps.push(Representation {
                id,
                bandwidth,
                codecs: rep_attrs.get("codecs").cloned().unwrap_or_default(),
                mime_type: rep_attrs
                    .get("mimeType")
                    .or_else(|| aset_attrs.get("mimeType"))
                    .cloned()
                    .unwrap_or_default(),
                width: rep_attrs
                    .get("width")
                    .and_then(|w| w.parse::<u32>().ok())
                    .unwrap_or(0),
                height: rep_attrs
                    .get("height")
                    .and_then(|h| h.parse::<u32>().ok())
                    .unwrap_or(0),
                base_url: rep_base,
                segment_template,
                segments,
            });
        }
        if !reps.is_empty() {
            adaptation_sets.push(AdaptationSet {
                content_type,
                representations: reps,
            });
        }
    }

    Ok(Mpd {
        duration_secs,
        min_buffer_secs,
        adaptation_sets,
    })
}

fn parse_template(attrs: &HashMap<String, String>) -> SegmentTemplate {
    SegmentTemplate {
        media: attrs.get("media").cloned().unwrap_or_default(),
        initialization: attrs.get("initialization").cloned().unwrap_or_default(),
        timescale: attrs
            .get("timescale")
            .and_then(|t| t.parse::<u64>().ok())
            .unwrap_or(1),
        start_number: attrs
            .get("startNumber")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(1),
        duration: attrs
            .get("duration")
            .and_then(|d| d.parse::<u64>().ok())
            .unwrap_or(0),
    }
}

// --- ABR selection ---------------------------------------------------------

/// Adaptive bitrate policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbrPolicy {
    /// Prefer the highest-representation that fits the budget (quality-first).
    MaximizeQuality,
    /// Safest representation strictly under the budget (stall-free).
    Conservative,
}

/// Pick the best representation for `content_type` given an available-bandwidth
/// budget (bits/sec). Returns `None` if there are no representations of that
/// type.
pub fn select_representation<'a>(
    mpd: &'a Mpd,
    content_type: &str,
    bandwidth_budget: u64,
    policy: AbrPolicy,
) -> Option<&'a Representation> {
    let mut reps: Vec<&'a Representation> = mpd.by_type(content_type);
    if reps.is_empty() {
        return None;
    }
    reps.sort_by_key(|r| r.bandwidth);
    match policy {
        AbrPolicy::Conservative => {
            let best = reps
                .iter()
                .rev()
                .find(|r| r.bandwidth <= bandwidth_budget)
                .copied();
            best.or_else(|| reps.first().copied())
        }
        AbrPolicy::MaximizeQuality => {
            let best = reps
                .iter()
                .filter(|r| r.bandwidth <= bandwidth_budget)
                .max_by_key(|r| r.bandwidth)
                .copied();
            // Budget below the lowest ladder rung: emit the lowest rather than
            // fail, so startup can at least begin buffering.
            best.or_else(|| reps.first().copied())
        }
    }
}

/// A DASH client driving ABR + segment enumeration for a single adaptation set.
#[derive(Debug, Clone)]
pub struct DashClient {
    mpd: Mpd,
    content_type: String,
    bandwidth_budget: u64,
    policy: AbrPolicy,
}

impl DashClient {
    pub fn new(mpd: Mpd, content_type: &str, bandwidth_budget: u64) -> Self {
        DashClient {
            mpd,
            content_type: content_type.to_string(),
            bandwidth_budget,
            policy: AbrPolicy::MaximizeQuality,
        }
    }

    pub fn with_policy(mut self, policy: AbrPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Re-evaluate the current network budget (called periodically by the host).
    pub fn set_bandwidth_budget(&mut self, budget: u64) {
        self.bandwidth_budget = budget;
    }

    /// The representation selected for the current budget.
    pub fn current(&self) -> Option<Representation> {
        select_representation(&self.mpd, &self.content_type, self.bandwidth_budget, self.policy)
            .cloned()
    }

    /// All segment URLs for the currently-selected representation.
    pub fn segment_plan(&self) -> Vec<Segment> {
        let Some(rep) = select_representation(
            &self.mpd,
            &self.content_type,
            self.bandwidth_budget,
            self.policy,
        ) else {
            return Vec::new();
        };
        if !rep.segments.is_empty() {
            return rep.segments.clone();
        }
        let count = self.mpd.segment_count(&rep);
        rep.templated_segments(count)
    }
}

// --- helpers ---------------------------------------------------------------

fn substitute(template: &str, rep_id: &str, bandwidth: u64, number: u64, time: u64) -> String {
    template
        .replace("$RepresentationID$", rep_id)
        .replace("$Number%01d$", &number.to_string())
        .replace("$Number$", &number.to_string())
        .replace("$Bandwidth%01d$", &bandwidth.to_string())
        .replace("$Bandwidth$", &bandwidth.to_string())
        .replace("$Time$", &time.to_string())
        // Drop residual unresolved placeholders so URLs stay valid-ish.
        .replace("$", "")
}

fn ensure_trailing_slash(s: &str) -> String {
    if s.ends_with('/') {
        s.to_string()
    } else {
        format!("{s}/")
    }
}

fn mime_to_type(mime: &str) -> String {
    if mime.starts_with("video/") {
        "video".into()
    } else if mime.starts_with("audio/") {
        "audio".into()
    } else if mime.starts_with("text/") || mime.starts_with("application/") {
        "text".into()
    } else {
        mime.to_string()
    }
}

/// Parse an ISO-8601 duration used by DASH (`PT12.5S`, `PT1M30S`, `PT0H0M10S`).
/// Returns seconds. Non-durations (or unparseable) yield `0.0`.
fn parse_duration_attr(v: Option<&String>) -> f64 {
    let Some(v) = v else { return 0.0 };
    let v = v.trim();
    if !v.starts_with("PT") {
        return v.parse::<f64>().unwrap_or(0.0);
    }
    let body = &v[2..];
    let mut total = 0.0;
    let mut num = String::new();
    for c in body.chars() {
        if c.is_ascii_digit() || c == '.' {
            num.push(c);
        } else {
            let value = num.parse::<f64>().unwrap_or(0.0);
            num.clear();
            match c {
                'H' => total += value * 3600.0,
                'M' => total += value * 60.0,
                'S' => total += value,
                _ => {}
            }
        }
    }
    total
}

/// Find the first occurrence of `tag` and return its attributes + inner text.
fn find_element<'a>(xml: &'a str, tag: &str) -> Option<(HashMap<String, String>, String)> {
    let open = format!("<{tag}");
    let start = xml.find(&open)?;
    // Find the matching '>' that closes the opening tag (skip self-closing).
    let mut i = start + open.len();
    let mut in_quote = false;
    let mut quote_ch = ' ';
    while i < xml.len() {
        let c = xml.as_bytes()[i] as char;
        if in_quote {
            if c == quote_ch {
                in_quote = false;
            }
        } else if c == '"' || c == '\'' {
            in_quote = true;
            quote_ch = c;
        } else if c == '>' {
            break;
        }
        i += 1;
    }
    if i >= xml.len() {
        return None;
    }
    let tag_text = &xml[start..=i];
    let attrs = parse_attrs(tag_text);
    let self_closing = tag_text.trim_end().ends_with("/>");
    if self_closing {
        return Some((attrs, String::new()));
    }
    let close = format!("</{tag}>");
    let end = xml[i + 1..].find(&close).map(|e| i + 1 + e)?;
    let inner = xml[i + 1..end].to_string();
    Some((attrs, inner))
}

/// Find all non-nested occurrences of `tag`.
fn find_all<'a>(xml: &'a str, tag: &str) -> Vec<(HashMap<String, String>, String)> {
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some((attrs, inner)) = find_element(&xml[pos..], tag) {
        // Re-locate the consumed slice to advance the scan past this element.
        out.push((attrs, inner.clone()));
        let consumed = element_span(&xml[pos..], tag);
        eprintln!("DBG find_all tag={tag} pos={pos} consumed={consumed} next={:?}", &xml[pos..pos+consumed.min(25)].replace('\n'," "));
        pos += consumed;
        if consumed == 0 {
            break;
        }
    }
    out
}

/// Length in bytes of the full `<tag>…</tag>` (or self-closing) element at the
/// start of `xml`.
fn element_span(xml: &str, tag: &str) -> usize {
    let open = format!("<{tag}");
    let start = match xml.find(&open) {
        Some(s) => s,
        None => return 0,
    };
    let mut i = start + open.len();
    let mut in_quote = false;
    let mut quote_ch = ' ';
    while i < xml.len() {
        let c = xml.as_bytes()[i] as char;
        if in_quote {
            if c == quote_ch {
                in_quote = false;
            }
        } else if c == '"' || c == '\'' {
            in_quote = true;
            quote_ch = c;
        } else if c == '>' {
            break;
        }
        i += 1;
    }
    if i >= xml.len() {
        return 0;
    }
    let tag_text = &xml[start..=i];
    if tag_text.trim_end().ends_with("/>") {
        return i + 1 - start;
    }
    let close = format!("</{tag}>");
    match xml[i + 1..].find(&close) {
        Some(e) => i + 1 + e + close.len() - start,
        None => 0,
    }
}

/// Parse attributes from a tag's raw text (handles `"` and `'` quoting).
fn parse_attrs(tag_text: &str) -> HashMap<String, String> {
    let mut attrs = HashMap::new();
    let bytes = tag_text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Scan for an attribute name: [A-Za-z_][\w:-]*
        if !(bytes[i].is_ascii_alphabetic() || bytes[i] == b'_') {
            i += 1;
            continue;
        }
        let name_start = i;
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'-' || bytes[i] == b':') {
            i += 1;
        }
        let name = tag_text[name_start..i].to_string();
        // Skip whitespace and expect '='.
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let q = bytes[i] as char;
        if q == '"' || q == '\'' {
            let qch = q;
            i += 1;
            let val_start = i;
            while i < bytes.len() && (bytes[i] as char) != qch {
                i += 1;
            }
            let val = tag_text[val_start..i].to_string();
            attrs.insert(name, val);
            i += 1;
        } else {
            let val_start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' && bytes[i] != b'/' {
                i += 1;
            }
            let val = tag_text[val_start..i].to_string();
            attrs.insert(name, val);
        }
    }
    attrs
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0"?>
<MPD mediaPresentationDuration="PT10S" minBufferTime="PT2S">
  <BaseURL>https://cdn.example.com/vod/</BaseURL>
  <AdaptationSet contentType="video" mimeType="video/mp4">
    <SegmentTemplate media="$RepresentationID$/$Number$.m4s" initialization="$RepresentationID$/init.mp4" timescale="1000" duration="2000" startNumber="1"/>
    <Representation id="v1" bandwidth="500000" codecs="avc1.4d400c" width="640" height="360"/>
    <Representation id="v2" bandwidth="1200000" codecs="avc1.4d4015" width="1280" height="720"/>
    <Representation id="v3" bandwidth="3000000" codecs="avc1.4d401e" width="1920" height="1080"/>
  </AdaptationSet>
  <AdaptationSet contentType="audio" mimeType="audio/mp4">
    <Representation id="a1" bandwidth="96000" codecs="mp4a.40.2"/>
  </AdaptationSet>
</MPD>"#;

    #[test]
    fn parses_manifest_structure() {
        let mpd = parse_mpd(SAMPLE).expect("parse");
        for a in &mpd.adaptation_sets {
            eprintln!("aset type={:?} reps={}", a.content_type, a.representations.len());
            for r in &a.representations {
                eprintln!("   rep id={} ct={}", r.id, r.mime_type);
            }
        }
        assert_eq!(mpd.duration_secs, 10.0);
        assert_eq!(mpd.min_buffer_secs, 2.0);
        assert_eq!(mpd.adaptation_sets.len(), 2);
        let video = mpd.by_type("video");
        assert_eq!(video.len(), 3);
        assert_eq!(video[0].bandwidth, 500_000);
        assert_eq!(video[2].resolution(), (1920, 1080));
    }

    #[test]
    fn template_substitution_builds_segment_urls() {
        let mpd = parse_mpd(SAMPLE).unwrap();
        let v2 = mpd.by_type("video")[1].clone();
        let segs = v2.templated_segments(2);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].url, "https://cdn.example.com/vod/v2/1.m4s");
        assert_eq!(segs[1].url, "https://cdn.example.com/vod/v2/2.m4s");
        assert_eq!(segs[0].duration_secs, 2.0);
        assert_eq!(v2.initialization_url(), Some("https://cdn.example.com/vod/v2/init.mp4".into()));
    }

    #[test]
    fn abr_picks_best_within_budget() {
        let mpd = parse_mpd(SAMPLE).unwrap();
        let best = select_representation(&mpd, "video", 1_500_000, AbrPolicy::MaximizeQuality)
            .unwrap();
        assert_eq!(best.id, "v2");
        let cons = select_representation(&mpd, "video", 1_500_000, AbrPolicy::Conservative)
            .unwrap();
        assert_eq!(cons.id, "v2");
        let low = select_representation(&mpd, "video", 600_000, AbrPolicy::Conservative)
            .unwrap();
        assert_eq!(low.id, "v1");
    }

    #[test]
    fn abr_falls_back_to_lowest_below_ladder() {
        let mpd = parse_mpd(SAMPLE).unwrap();
        let pick = select_representation(&mpd, "video", 100, AbrPolicy::MaximizeQuality)
            .unwrap();
        assert_eq!(pick.id, "v1");
    }

    #[test]
    fn client_emits_segment_plan() {
        let mpd = parse_mpd(SAMPLE).unwrap();
        let client = DashClient::new(mpd, "video", 1_500_000);
        let plan = client.segment_plan();
        // 10s duration / 2s segments = 5 segments.
        assert_eq!(plan.len(), 5);
        assert!(plan[0].url.contains("v2/1.m4s"));
    }

    #[test]
    fn segment_list_is_parsed() {
        let xml = r#"<MPD mediaPresentationDuration="PT4S">
            <AdaptationSet contentType="video">
              <Representation id="r" bandwidth="800000">
                <SegmentList timescale="1000" duration="2000">
                  <SegmentURL media="seg1.m4s" d="2000"/>
                  <SegmentURL media="seg2.m4s" d="2000"/>
                </SegmentList>
              </Representation>
            </AdaptationSet>
          </MPD>"#;
        let mpd = parse_mpd(xml).unwrap();
        let rep = mpd.representations().next().unwrap();
        assert_eq!(rep.segments.len(), 2);
        assert_eq!(rep.segments[0].url, "seg1.m4s");
        assert_eq!(rep.segments[1].duration_secs, 2.0);
    }
}
