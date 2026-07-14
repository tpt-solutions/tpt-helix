//! Media decode backends — hardware (VA-API / Vulkan video) + software fallback
//! (spec §4 "Hardware Decode", Phase 1 "Media pipeline").
//!
//! The DASH client (`dash.rs`) produces segment URLs and an ABR ladder; the
//! *decode* half turns a fetched segment's bytes into frames. This module
//! models that as a [`DecoderBackend`] trait so the runtime can pick the best
//! available decoder per platform without the rest of the pipeline knowing
//! whether it is GPU-accelerated.
//!
//! Hardware paths (VA-API on Linux, Vulkan Video on cross-platform) are behind
//! the `hardware-decode` feature: enabling it selects the platform codec stack
//! at runtime via [`select_backend`]. Without the feature only the [`Software`]
//! backend is available, which keeps the crate buildable/testable on CI hosts
//! that have no GPU. The hardware backends are intentionally scaffolded — the
//! real codec integration is wired when building for a target with the relevant
//! driver present (tracked in TODO.md §"Media pipeline").

use crate::dash::Representation;

/// A decoder backend the media pipeline can drive.
pub trait DecoderBackend {
    /// Human-readable backend name (e.g. `"software"`, `"vaapi"`, `"vulkan"`).
    fn name(&self) -> &'static str;

    /// Whether this backend uses GPU-accelerated (hardware) decode.
    fn is_hardware(&self) -> bool;

    /// Suitability score for `rep` (higher = better). A backend that cannot
    /// decode `rep`'s codec returns `None`.
    fn score(&self, rep: &Representation) -> Option<u32>;

    /// Begin decoding a segment of `len` bytes (placeholder for the real
    /// decode loop). Returns a frame count estimate for benchmarking.
    fn decode_segment(&mut self, rep: &Representation, len: usize) -> DecodeOutcome;
}

/// Outcome of decoding one segment (placeholder stats; real backends would
/// report actual frame counts / decode time).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeOutcome {
    pub frames: u64,
    pub bytes: usize,
}

/// The decode backends the runtime can select between.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Software,
    /// VA-API (Linux/Intel/AMD). Only available with `hardware-decode`.
    Vaapi,
    /// Vulkan Video (cross-platform GPU). Only available with `hardware-decode`.
    Vulkan,
}

impl BackendKind {
    pub fn name(self) -> &'static str {
        match self {
            BackendKind::Software => "software",
            BackendKind::Vaapi => "vaapi",
            BackendKind::Vulkan => "vulkan",
        }
    }

    pub fn is_hardware(self) -> bool {
        !matches!(self, BackendKind::Software)
    }
}

/// The default pure-Rust/CPU decoder. Always available; no GPU required.
pub struct Software;

impl DecoderBackend for Software {
    fn name(&self) -> &'static str {
        "software"
    }

    fn is_hardware(&self) -> bool {
        false
    }

    fn score(&self, _rep: &Representation) -> Option<u32> {
        // Software can decode anything, but is the lowest-priority choice.
        Some(1)
    }

    fn decode_segment(&mut self, rep: &Representation, len: usize) -> DecodeOutcome {
        let dur = rep.segment_duration_secs();
        let fps = 30.0;
        let frames = if dur > 0.0 {
            (dur * fps).round() as u64
        } else {
            1
        };
        DecodeOutcome { frames, bytes: len }
    }
}

/// Probe which decode backends are actually usable in this build/target.
///
/// With the `hardware-decode` feature, this queries the platform codec stack
/// (VA-API on Linux, Vulkan Video where available) and reports what is present.
/// Without the feature, only [`BackendKind::Software`] is reported.
pub fn probe_backends() -> Vec<BackendKind> {
    let backends = vec![BackendKind::Software];
    #[cfg(feature = "hardware-decode")]
    {
        if crate::media_decode::platform::vaapi_available() {
            backends.push(BackendKind::Vaapi);
        }
        if crate::media_decode::platform::vulkan_video_available() {
            backends.push(BackendKind::Vulkan);
        }
    }
    backends
}

/// Select the best backend for `rep` from the available set (hardware wins,
/// ties broken by capability score). Falls back to [`Software`] when nothing
/// with higher priority is usable.
pub fn select_backend(rep: &Representation) -> Box<dyn DecoderBackend> {
    let mut candidates: Vec<(BackendKind, u32)> = probe_backends()
        .into_iter()
        .filter_map(|k| {
            let backend: Box<dyn DecoderBackend> = match k {
                BackendKind::Software => Box::new(Software),
                BackendKind::Vaapi => Box::new(HardwareBackend::vaapi()),
                BackendKind::Vulkan => Box::new(HardwareBackend::vulkan()),
            };
            backend.score(rep).map(|s| (k, s))
        })
        .collect();
    // Prefer hardware, then higher score.
    candidates.sort_by(|a, b| {
        b.0.is_hardware()
            .cmp(&a.0.is_hardware())
            .then(b.1.cmp(&a.1))
    });
    match candidates.first().map(|(k, _)| *k) {
        Some(BackendKind::Vaapi) => Box::new(HardwareBackend::vaapi()),
        Some(BackendKind::Vulkan) => Box::new(HardwareBackend::vulkan()),
        _ => Box::new(Software),
    }
}

/// Hardware decode backend (VA-API / Vulkan Video). Constructed only when the
/// `hardware-decode` feature is enabled; otherwise this is a documented
/// placeholder that records the intended path without touching GPU drivers.
pub struct HardwareBackend {
    kind: BackendKind,
}

impl HardwareBackend {
    fn vaapi() -> Self {
        HardwareBackend {
            kind: BackendKind::Vaapi,
        }
    }
    fn vulkan() -> Self {
        HardwareBackend {
            kind: BackendKind::Vulkan,
        }
    }
}

impl DecoderBackend for HardwareBackend {
    fn name(&self) -> &'static str {
        self.kind.name()
    }

    fn is_hardware(&self) -> bool {
        true
    }

    fn score(&self, rep: &Representation) -> Option<u32> {
        // Hardware decoders prefer higher resolutions (they are the reason we
        // have them). Unavailable in a non-`hardware-decode` build.
        if !cfg!(feature = "hardware-decode") {
            return None;
        }
        let (w, h) = rep.resolution();
        Some(100 + w / 10 + h / 10)
    }

    fn decode_segment(&mut self, rep: &Representation, len: usize) -> DecodeOutcome {
        let dur = rep.segment_duration_secs();
        let frames = if dur > 0.0 {
            (dur * 30.0).round() as u64
        } else {
            1
        };
        DecodeOutcome { frames, bytes: len }
    }
}

/// Platform capability probes for hardware decode. Compiled only under the
/// `hardware-decode` feature; the default build reports no hardware decoders
/// (correct on headless CI).
pub mod platform {
    /// True if a VA-API driver is resolved on this Linux host.
    pub fn vaapi_available() -> bool {
        #[cfg(all(feature = "hardware-decode", target_os = "linux"))]
        {
            // A real build would call into libva here. We conservatively report
            // availability based on the presence of the VA-API device path.
            std::path::Path::new("/dev/dri").exists()
        }
        #[cfg(not(all(feature = "hardware-decode", target_os = "linux")))]
        {
            false
        }
    }

    /// True if a Vulkan instance exposes the video-decode extension.
    pub fn vulkan_video_available() -> bool {
        #[cfg(feature = "hardware-decode")]
        {
            // A real build would enumerate Vulkan physical devices for
            // `VK_KHR_video_decode`. Reported false until wired.
            false
        }
        #[cfg(not(feature = "hardware-decode"))]
        {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dash::{parse_mpd, AbrPolicy};

    const MPD: &str = r#"<MPD mediaPresentationDuration="PT4S">
      <AdaptationSet contentType="video">
        <SegmentTemplate media="v/$Number$.m4s" timescale="1000" duration="2000"/>
        <Representation id="v2" bandwidth="1200000" codecs="avc1.4d4015" width="1280" height="720"/>
      </AdaptationSet>
    </MPD>"#;

    #[test]
    fn software_backend_always_available_and_scores() {
        let mpd = parse_mpd(MPD).unwrap();
        let rep = crate::dash::select_representation(&mpd, "video", 2_000_000, AbrPolicy::MaximizeQuality).unwrap();
        let mut sw = Software;
        assert!(!sw.is_hardware());
        assert_eq!(sw.score(&rep), Some(1));
        let out = sw.decode_segment(&rep, 4096);
        assert_eq!(out.bytes, 4096);
        assert!(out.frames >= 1);
    }

    #[test]
    fn select_backend_returns_software_without_hw_feature() {
        let mpd = parse_mpd(MPD).unwrap();
        let rep = crate::dash::select_representation(&mpd, "video", 2_000_000, AbrPolicy::MaximizeQuality).unwrap();
        let backend = select_backend(&rep);
        assert_eq!(backend.name(), "software");
        assert!(!backend.is_hardware());
        // Hardware backend scores None without the feature, so it is not picked.
        assert_eq!(HardwareBackend::vaapi().score(&rep), None);
    }

    #[test]
    fn probe_backends_includes_software() {
        assert!(probe_backends().contains(&BackendKind::Software));
    }
}
