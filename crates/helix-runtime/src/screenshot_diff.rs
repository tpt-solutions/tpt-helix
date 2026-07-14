//! Visual regression checking (Stage S3: Validate, spec §6.3).
//!
//! Given two RGBA frames produced by [`crate::software_raster`], compute how
//! different they are and, when they differ, produce a *diff image* that
//! highlights every changed pixel so a reviewer can see what regressed. This
//! is the headless stand-in for pixel-comparing a real GPU-rendered frame
//! against a committed baseline in CI.
//!
//! The matching metric is deliberately simple and explainable: the fraction of
//! pixels that changed in any channel, plus the worst single-channel delta. A
//! migration/CI gate compares the changed-fraction against a small threshold.

use image::{Rgba, RgbaImage};

/// Summary of comparing an `actual` frame against an `expected` baseline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DiffReport {
    /// Fraction of pixels (0.0–1.0) that differ in at least one channel.
    pub changed_ratio: f32,
    /// Largest absolute single-channel difference across all pixels (0–255).
    pub max_channel_delta: u8,
    /// Mean absolute per-channel difference across all pixels (0.0–255.0).
    pub mean_channel_delta: f32,
    /// `true` when the frames are the same size and `changed_ratio == 0`.
    pub identical: bool,
}

/// Compare `actual` to `expected`. Returns `None` if the frames differ in size
/// (a size change is itself a regression and should be handled by the caller).
pub fn compare(actual: &RgbaImage, expected: &RgbaImage) -> Option<DiffReport> {
    if actual.width() != expected.width() || actual.height() != expected.height() {
        return None;
    }
    let mut changed = 0u64;
    let mut max_delta: u8 = 0;
    let mut sum_delta: u64 = 0;
    let total = (actual.width() as u64) * (actual.height() as u64);

    for (a, e) in actual.pixels().zip(expected.pixels()) {
        let mut px_changed = false;
        for c in 0..4 {
            let d = a.0[c].abs_diff(e.0[c]);
            if d > 0 {
                px_changed = true;
            }
            if d > max_delta {
                max_delta = d;
            }
            sum_delta += d as u64;
        }
        if px_changed {
            changed += 1;
        }
    }

    let identical = changed == 0;
    Some(DiffReport {
        changed_ratio: changed as f32 / total as f32,
        max_channel_delta: max_delta,
        mean_channel_delta: sum_delta as f32 / total as f32,
        identical,
    })
}

/// Build a diff image for human review: unchanged pixels are dimmed, changed
/// pixels are painted solid red. `expected` provides the dimmed backdrop.
pub fn diff_image(actual: &RgbaImage, expected: &RgbaImage) -> Option<RgbaImage> {
    if actual.width() != expected.width() || actual.height() != expected.height() {
        return None;
    }
    let mut out = RgbaImage::new(actual.width(), actual.height());
    for (i, (a, e)) in actual.pixels().zip(expected.pixels()).enumerate() {
        let changed = (0..4).any(|c| a.0[c] != e.0[c]);
        if changed {
            out.put_pixel(
                (i as u32) % actual.width(),
                (i as u32) / actual.width(),
                Rgba([255, 0, 0, 255]),
            );
        } else {
            // Dim the unchanged backdrop so the red pops.
            out.put_pixel(
                (i as u32) % actual.width(),
                (i as u32) / actual.width(),
                Rgba([e.0[0] / 3, e.0[1] / 3, e.0[2] / 3, e.0[3]]),
            );
        }
    }
    Some(out)
}

/// Assert that `actual` matches `expected` within `max_changed_ratio`
/// (a 0.0–1.0 fraction of pixels allowed to differ). Returns the [`DiffReport`]
/// on success so callers can log it; panics (with a rendered diff summary) on
/// exceeding the threshold.
pub fn assert_within(
    actual: &RgbaImage,
    expected: &RgbaImage,
    max_changed_ratio: f32,
) -> DiffReport {
    let report = compare(actual, expected).expect("frames must be the same size");
    assert!(
        report.changed_ratio <= max_changed_ratio,
        "visual regression: changed_ratio {:.4} exceeds threshold {:.4} (max delta {}, mean {:4})",
        report.changed_ratio,
        max_changed_ratio,
        report.max_channel_delta,
        report.mean_channel_delta
    );
    report
}

/// Like [`compare`] but returns the inclusive bounding box of all changed
/// pixels `(x0, y0, x1, y1)` in image coordinates, or `None` when the frames
/// are identical or differ in size. The bounds localize a regression to a
/// region of the frame, which lets a CI gate report *where* a composed
/// (multi-component) UI changed rather than only that *something* changed.
pub fn changed_bounds(actual: &RgbaImage, expected: &RgbaImage) -> Option<(u32, u32, u32, u32)> {
    if actual.width() != expected.width() || actual.height() != expected.height() {
        return None;
    }
    let mut x0 = actual.width();
    let mut y0 = actual.height();
    let mut x1 = 0u32;
    let mut y1 = 0u32;
    let mut any = false;
    for (i, (a, e)) in actual.pixels().zip(expected.pixels()).enumerate() {
        if (0..4).any(|c| a.0[c] != e.0[c]) {
            let x = (i as u32) % actual.width();
            let y = (i as u32) / actual.width();
            if x < x0 {
                x0 = x;
            }
            if y < y0 {
                y0 = y;
            }
            if x > x1 {
                x1 = x;
            }
            if y > y1 {
                y1 = y;
            }
            any = true;
        }
    }
    if !any {
        None
    } else {
        Some((x0, y0, x1, y1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn solid(w: u32, h: u32, c: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba(c))
    }

    #[test]
    fn identical_frames_report_zero() {
        let a = solid(4, 4, [10, 20, 30, 255]);
        let b = solid(4, 4, [10, 20, 30, 255]);
        let r = compare(&a, &b).expect("same size");
        assert!(r.identical);
        assert_eq!(r.changed_ratio, 0.0);
        assert_eq!(r.max_channel_delta, 0);
    }

    #[test]
    fn differing_frames_report_nonzero() {
        let a = solid(4, 4, [10, 20, 30, 255]);
        let b = solid(4, 4, [200, 20, 30, 255]);
        let r = compare(&a, &b).expect("same size");
        assert!(!r.identical);
        assert_eq!(r.changed_ratio, 1.0);
        assert_eq!(r.max_channel_delta, 190);
    }

    #[test]
    fn size_mismatch_is_none() {
        assert!(compare(&solid(4, 4, [0, 0, 0, 0]), &solid(5, 5, [0, 0, 0, 0])).is_none());
    }

    #[test]
    fn diff_image_marks_changes_red() {
        let mut a = solid(2, 2, [0, 0, 0, 0]);
        a.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
        let b = solid(2, 2, [0, 0, 0, 0]);
        let d = diff_image(&a, &b).expect("same size");
        assert_eq!(d.get_pixel(0, 0), &Rgba([255, 0, 0, 255]));
        // Unchanged pixel is dimmed, not red.
        assert_ne!(d.get_pixel(1, 1), &Rgba([255, 0, 0, 255]));
    }

    #[test]
    fn assert_within_passes_under_threshold() {
        let a = solid(10, 10, [0, 0, 0, 0]);
        let b = solid(10, 10, [0, 0, 0, 0]);
        assert_within(&a, &b, 0.01);
    }

    #[test]
    fn changed_bounds_locates_single_pixel_change() {
        let mut a = solid(8, 8, [0, 0, 0, 0]);
        let b = solid(8, 8, [0, 0, 0, 0]);
        a.put_pixel(3, 5, Rgba([255, 0, 0, 255]));
        let bounds = changed_bounds(&a, &b).expect("one pixel changed");
        assert_eq!(bounds, (3, 5, 3, 5));
    }

    #[test]
    fn changed_bounds_spans_a_moved_element() {
        let mut baseline = solid(40, 40, [240, 240, 240, 255]);
        let mut regressed = solid(40, 40, [240, 240, 240, 255]);
        // Baseline: a 10x10 red block at (5,5). Regressed: same block at (25,5).
        for y in 5..15 {
            for x in 5..15 {
                baseline.put_pixel(x, y, Rgba([200, 0, 0, 255]));
            }
            for x in 25..35 {
                regressed.put_pixel(x, y, Rgba([200, 0, 0, 255]));
            }
        }
        let bounds = changed_bounds(&regressed, &baseline).expect("change present");
        // Both the vacated region (5..15) and the new region (25..35) are hit,
        // so the bounding box spans the full horizontal move.
        assert_eq!(bounds, (5, 5, 34, 14));
    }

    #[test]
    fn changed_bounds_none_when_identical_or_sized_mismatch() {
        let a = solid(6, 6, [1, 2, 3, 4]);
        let b = solid(6, 6, [1, 2, 3, 4]);
        assert!(changed_bounds(&a, &b).is_none());
        assert!(changed_bounds(&a, &solid(7, 7, [1, 2, 3, 4])).is_none());
    }
}
