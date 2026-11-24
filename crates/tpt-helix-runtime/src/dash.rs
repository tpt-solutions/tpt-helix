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

use roxmltree::Node;

/// Upper bound on XML nodes parsed from one manifest. Bounds parser memory for
/// hostile input independently of the segment and representation caps.
pub const MAX_MPD_XML_NODES: u32 = 1_000_000;

/// Upper bound on segments the client will enumerate for one representation.
/// A manifest that declares more is rejected at parse time, and segment-count
/// math is clamped to this value, so a hostile `mediaPresentationDuration` or
/// `SegmentList` cannot force unbounded allocation.
pub const MAX_SEGMENTS_PER_REPRESENTATION: u64 = 100_000;

/// Upper bound on representations across all adaptation sets of one manifest.
pub const MAX_REPRESENTATIONS: usize = 1_000;

/// Errors produced while parsing an MPD manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DashError {
    /// A required element/attribute was missing or malformed, or a manifest
    /// exceeded a resource bound.
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
        let base = if self.base_url.is_empty() {
            String::new()
        } else {
            ensure_trailing_slash(&self.base_url)
        };
        Some(format!(
            "{}{}",
            base,
            substitute(
                &t.initialization,
                &self.id,
                self.bandwidth,
                t.start_number,
                0
            )
        ))
    }

    /// Synthesize the segment URLs for the templated portion of this
    /// representation, up to `count` segments starting at `start_number`.
    pub fn templated_segments(&self, count: u64) -> Vec<Segment> {
        let Some(t) = &self.segment_template else {
            return Vec::new();
        };
        let count = count.min(MAX_SEGMENTS_PER_REPRESENTATION);
        let dur = self.segment_duration_secs();
        let base = if self.base_url.is_empty() {
            String::new()
        } else {
            ensure_trailing_slash(&self.base_url)
        };
        let start = if t.start_number == 0 {
            1
        } else {
            t.start_number
        };
        (0..count)
            .map(|i| {
                let n = start.saturating_add(i);
                let time = n.saturating_sub(start).saturating_mul(t.duration);
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
            return (rep.segments.len() as u64).min(MAX_SEGMENTS_PER_REPRESENTATION);
        }
        let raw = (self.duration_secs / dur).ceil();
        if raw.is_nan() {
            return 0;
        }
        // `f64 as u64` saturates, but clamp explicitly so the bound is visible.
        (raw.min(MAX_SEGMENTS_PER_REPRESENTATION as f64)) as u64
    }
}

// --- parsing ---------------------------------------------------------------

/// Parse an MPD manifest string into a client model.
///
/// Parsed with a real XML parser (`roxmltree`, DTDs disallowed, node count
/// bounded by [`MAX_MPD_XML_NODES`]). Tolerant of the common DASH shapes
/// (SegmentTemplate, SegmentList, BaseURL inheritance). Unknown elements are
/// skipped. Malformed XML is rejected.
pub fn parse_mpd(xml: &str) -> Result<Mpd, DashError> {
    let opts = roxmltree::ParsingOptions {
        allow_dtd: false,
        nodes_limit: MAX_MPD_XML_NODES,
    };
    let doc = roxmltree::Document::parse_with_options(xml, opts)
        .map_err(|e| DashError::Malformed(format!("invalid MPD XML: {e}")))?;
    let root = doc.root_element();
    if root.tag_name().name() != "MPD" {
        return Err(DashError::Malformed("no MPD root element".into()));
    }

    let duration_secs = parse_duration_attr(root.attribute("mediaPresentationDuration"));
    let min_buffer_secs = parse_duration_attr(root.attribute("minBufferTime"));
    let base_url = first_child_named(root, "BaseURL")
        .map(text_of)
        .unwrap_or_default();

    let mut adaptation_sets = Vec::new();
    let mut total_reps = 0usize;
    for aset in descendants_named(root, "AdaptationSet") {
        let content_type = aset
            .attribute("contentType")
            .map(str::to_string)
            .or_else(|| aset.attribute("mimeType").map(mime_to_type))
            .unwrap_or_default();
        let set_base = first_child_named(aset, "BaseURL")
            .map(text_of)
            .unwrap_or_else(|| base_url.clone());

        let mut reps = Vec::new();
        for rep in descendants_named(aset, "Representation") {
            total_reps += 1;
            if total_reps > MAX_REPRESENTATIONS {
                return Err(DashError::Malformed(format!(
                    "manifest exceeds {MAX_REPRESENTATIONS} representations"
                )));
            }
            let id = rep.attribute("id").unwrap_or("<no-id>").to_string();
            let bandwidth = rep
                .attribute("bandwidth")
                .and_then(|b| b.parse::<u64>().ok())
                .unwrap_or(0);
            let rep_base = first_child_named(rep, "BaseURL")
                .map(text_of)
                .unwrap_or_else(|| set_base.clone());
            let segment_template = first_child_named(rep, "SegmentTemplate")
                .or_else(|| first_child_named(aset, "SegmentTemplate"))
                .map(parse_template);

            let mut segments = Vec::new();
            let segment_list = first_child_named(rep, "SegmentList")
                .or_else(|| first_child_named(aset, "SegmentList"));
            if let Some(list) = segment_list {
                for seg in children_named(list, "SegmentURL") {
                    if segments.len() as u64 >= MAX_SEGMENTS_PER_REPRESENTATION {
                        return Err(DashError::Malformed(format!(
                            "representation {id} exceeds {MAX_SEGMENTS_PER_REPRESENTATION} segments"
                        )));
                    }
                    let url = seg
                        .attribute("media")
                        .map(str::to_string)
                        .or_else(|| {
                            first_child_named(seg, "BaseURL").map(|b| {
                                format!("{}{}", ensure_trailing_slash(&rep_base), text_of(b))
                            })
                        })
                        .unwrap_or_default();
                    let dur = seg
                        .attribute("d")
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
                codecs: rep.attribute("codecs").unwrap_or_default().to_string(),
                mime_type: rep
                    .attribute("mimeType")
                    .or_else(|| aset.attribute("mimeType"))
                    .unwrap_or_default()
                    .to_string(),
                width: rep
                    .attribute("width")
                    .and_then(|w| w.parse::<u32>().ok())
                    .unwrap_or(0),
                height: rep
                    .attribute("height")
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

/// Direct element children of `node` named `name` (namespace-agnostic).
fn children_named<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Vec<Node<'a, 'i>> {
    node.children()
        .filter(|n| n.is_element() && n.tag_name().name() == name)
        .collect()
}

fn first_child_named<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    node.children()
        .find(|n| n.is_element() && n.tag_name().name() == name)
}

/// All element descendants of `node` named `name`, excluding `node` itself, in
/// document order.
fn descendants_named<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Vec<Node<'a, 'i>> {
    node.descendants()
        .filter(|n| n.is_element() && n.id() != node.id() && n.tag_name().name() == name)
        .collect()
}

/// Trimmed text content of an element (empty if it has none).
fn text_of(node: Node<'_, '_>) -> String {
    node.text().unwrap_or("").trim().to_string()
}

fn parse_template(node: Node<'_, '_>) -> SegmentTemplate {
    SegmentTemplate {
        media: node.attribute("media").unwrap_or_default().to_string(),
        initialization: node
            .attribute("initialization")
            .unwrap_or_default()
            .to_string(),
        timescale: node
            .attribute("timescale")
            .and_then(|t| t.parse::<u64>().ok())
            .unwrap_or(1),
        start_number: node
            .attribute("startNumber")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(1),
        duration: node
            .attribute("duration")
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
        select_representation(
            &self.mpd,
            &self.content_type,
            self.bandwidth_budget,
            self.policy,
        )
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
        let count = self.mpd.segment_count(rep);
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
fn parse_duration_attr(v: Option<&str>) -> f64 {
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
            eprintln!(
                "aset type={:?} reps={}",
                a.content_type,
                a.representations.len()
            );
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
        assert_eq!(
            v2.initialization_url(),
            Some("https://cdn.example.com/vod/v2/init.mp4".into())
        );
    }

    #[test]
    fn abr_picks_best_within_budget() {
        let mpd = parse_mpd(SAMPLE).unwrap();
        let best =
            select_representation(&mpd, "video", 1_500_000, AbrPolicy::MaximizeQuality).unwrap();
        assert_eq!(best.id, "v2");
        let cons =
            select_representation(&mpd, "video", 1_500_000, AbrPolicy::Conservative).unwrap();
        assert_eq!(cons.id, "v2");
        let low = select_representation(&mpd, "video", 600_000, AbrPolicy::Conservative).unwrap();
        assert_eq!(low.id, "v1");
    }

    #[test]
    fn abr_falls_back_to_lowest_below_ladder() {
        let mpd = parse_mpd(SAMPLE).unwrap();
        let pick = select_representation(&mpd, "video", 100, AbrPolicy::MaximizeQuality).unwrap();
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

    #[test]
    fn segment_list_over_limit_is_rejected() {
        let mut xml = String::from(
            r#"<MPD mediaPresentationDuration="PT4S"><AdaptationSet contentType="video">
              <Representation id="r" bandwidth="800000"><SegmentList>"#,
        );
        for _ in 0..=MAX_SEGMENTS_PER_REPRESENTATION {
            xml.push_str(r#"<SegmentURL media="s.m4s"/>"#);
        }
        xml.push_str("</SegmentList></Representation></AdaptationSet></MPD>");
        assert!(matches!(parse_mpd(&xml), Err(DashError::Malformed(_))));
    }

    #[test]
    fn too_many_representations_is_rejected() {
        let mut xml = String::from(
            r#"<MPD mediaPresentationDuration="PT4S"><AdaptationSet contentType="video">"#,
        );
        for i in 0..=MAX_REPRESENTATIONS {
            xml.push_str(&format!(r#"<Representation id="r{i}" bandwidth="1"/>"#));
        }
        xml.push_str("</AdaptationSet></MPD>");
        assert!(matches!(parse_mpd(&xml), Err(DashError::Malformed(_))));
    }

    #[test]
    fn segment_count_is_clamped_for_hostile_durations() {
        // A huge presentation over a tiny segment duration would otherwise
        // request an enormous count.
        let mpd = Mpd {
            duration_secs: 1e300,
            min_buffer_secs: 0.0,
            adaptation_sets: vec![],
        };
        let rep = Representation {
            id: "r".into(),
            bandwidth: 1,
            codecs: String::new(),
            mime_type: String::new(),
            width: 0,
            height: 0,
            base_url: String::new(),
            segment_template: Some(SegmentTemplate {
                media: "m$Number$.m4s".into(),
                initialization: String::new(),
                timescale: 1,
                start_number: 1,
                duration: 1,
            }),
            segments: vec![],
        };
        let _ = rep.templated_segments(u64::MAX);
        assert_eq!(mpd.segment_count(&rep), MAX_SEGMENTS_PER_REPRESENTATION);
        // Non-finite durations never produce a count.
        let nan = Mpd {
            duration_secs: f64::NAN,
            ..mpd.clone()
        };
        assert_eq!(nan.segment_count(&rep), 0);
    }
}

#[cfg(test)]
mod xml_hardening_tests {
    use super::*;

    /// A BaseURL nested inside one AdaptationSet must not leak into a sibling
    /// AdaptationSet (the old subtree scan picked it up as the MPD-level base).
    #[test]
    fn nested_base_url_does_not_leak_to_sibling_adaptation_set() {
        let xml = r#"<MPD mediaPresentationDuration="PT4S">
            <AdaptationSet contentType="video">
              <BaseURL>https://a.example/</BaseURL>
              <Representation id="v1" bandwidth="1000"/>
            </AdaptationSet>
            <AdaptationSet contentType="audio">
              <Representation id="a1" bandwidth="2000"/>
            </AdaptationSet>
          </MPD>"#;
        let mpd = parse_mpd(xml).unwrap();
        let audio = mpd.by_type("audio");
        assert_eq!(audio.len(), 1);
        assert_eq!(audio[0].base_url, "");
        let video = mpd.by_type("video");
        assert_eq!(video[0].base_url, "https://a.example/");
    }

    /// `<RepresentationHint>` must not be counted as a `<Representation>`.
    #[test]
    fn element_names_are_matched_exactly() {
        let xml = r#"<MPD>
            <AdaptationSet contentType="video">
              <RepresentationHint id="ghost" bandwidth="1"/>
              <Representation id="v1" bandwidth="1000"/>
            </AdaptationSet>
          </MPD>"#;
        let mpd = parse_mpd(xml).unwrap();
        let reps: Vec<_> = mpd.representations().map(|r| r.id.clone()).collect();
        assert_eq!(reps, vec!["v1".to_string()]);
    }

    #[test]
    fn malformed_xml_is_rejected() {
        assert!(matches!(
            parse_mpd("<MPD><AdaptationSet>"),
            Err(DashError::Malformed(_))
        ));
        assert!(parse_mpd("<MPD><!DOCTYPE MPD [<!ENTITY x \"y\">]></MPD>").is_err());
    }

    /// The XML node budget bounds parser memory: a manifest past it is rejected
    /// rather than parsed in full.
    #[test]
    fn node_budget_rejects_oversized_manifest() {
        let mut xml = String::from("<MPD><AdaptationSet>");
        // Just past the 1_000_000-node budget (see `MAX_MPD_XML_NODES`).
        for _ in 0..1_000_010 {
            xml.push_str("<X/>");
        }
        xml.push_str("</AdaptationSet></MPD>");
        assert!(parse_mpd(&xml).is_err());
    }
}
