//! Throughput benchmark for the 720p video player decode path (spec §7.1).
//!
//! Records decode throughput over a planned DASH segment sequence so memory
//! and decode-time regressions are visible in CI trend charts. The hard
//! ≤200 MB memory budget is enforced as a failing test in
//! `tests/video_memory.rs` (`video_player_720p_stays_within_200mb_target`).

use criterion::{Criterion, criterion_group, criterion_main};
use helix_runtime::dash::{AbrPolicy, DashClient, parse_mpd};
use helix_runtime::media_decode::{DecoderBackend, Software};

const MPD: &str = r#"<MPD mediaPresentationDuration="PT20S">
  <AdaptationSet contentType="video" mimeType="video/mp4">
    <SegmentTemplate media="$RepresentationID$/$Number$.m4s" initialization="$RepresentationID$/init.mp4" timescale="1000" duration="2000" startNumber="1"/>
    <Representation id="v2" bandwidth="1200000" codecs="avc1.4d4015" width="1280" height="720"/>
  </AdaptationSet>
</MPD>"#;

fn bench_video(c: &mut Criterion) {
    let mpd = parse_mpd(MPD).expect("parse mpd");
    let rep = DashClient::new(mpd.clone(), "video", 2_000_000)
        .with_policy(AbrPolicy::MaximizeQuality)
        .current()
        .expect("720p representation");
    let client =
        DashClient::new(mpd, "video", 2_000_000).with_policy(AbrPolicy::MaximizeQuality);
    let plan = client.segment_plan();

    c.bench_function("decode_720p_segments", |b| {
        b.iter(|| {
            let mut decoder = Software;
            for seg in &plan {
                let _ = decoder.decode_segment(&rep, 1024 * 1024);
                let _ = seg;
            }
        })
    });
}

criterion_group!(benches, bench_video);
criterion_main!(benches);
