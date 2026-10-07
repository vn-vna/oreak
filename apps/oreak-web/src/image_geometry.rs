//! Native-testable placement math shared by Image Studio pointer interactions.
use oreak_core::ImagePlacement;

/// Translate or resize about the opposite corner; handle flags are (right, bottom).
pub fn drag_placement(
    initial: &ImagePlacement,
    delta: (f64, f64),
    handle: Option<(bool, bool)>,
    lock_aspect: bool,
) -> ImagePlacement {
    let mut next = initial.clone();
    let (dx, dy) = delta;
    if !dx.is_finite() || !dy.is_finite() || initial.validate().is_err() {
        return next;
    }
    if let Some((right, bottom)) = handle {
        let signed_dx = if right { dx } else { -dx };
        let signed_dy = if bottom { dy } else { -dy };
        let (width, height) = if lock_aspect {
            let horizontal = signed_dx / initial.width;
            let vertical = signed_dy / initial.height;
            let relative = if horizontal.abs() >= vertical.abs() {
                horizontal
            } else {
                vertical
            };
            let scale = (1.0 + relative).max(0.01 / initial.width.min(initial.height));
            (initial.width * scale, initial.height * scale)
        } else {
            (
                (initial.width + signed_dx).max(0.01),
                (initial.height + signed_dy).max(0.01),
            )
        };
        next.width = width;
        next.height = height;
        next.x = if right {
            initial.x
        } else {
            initial.x + initial.width - width
        };
        next.y = if bottom {
            initial.y
        } else {
            initial.y + initial.height - height
        };
    } else {
        next.x += dx;
        next.y += dy;
    }
    if next.validate().is_err() {
        initial.clone()
    } else {
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oreak_core::{ImageSampling, ImageTransparency};

    fn initial() -> ImagePlacement {
        ImagePlacement {
            x: 3.0,
            y: 5.0,
            width: 4.0,
            height: 2.0,
            sampling: ImageSampling::Area,
            pixelation: 2,
            resolution: Some(8),
            transparency: ImageTransparency::Erase,
        }
    }

    #[test]
    fn vertical_corner_resize_preserves_aspect_and_opposite_anchor() {
        let p = initial();
        for right in [false, true] {
            for bottom in [false, true] {
                let next = drag_placement(
                    &p,
                    (0.0, if bottom { 2.0 } else { -2.0 }),
                    Some((right, bottom)),
                    true,
                );
                assert_eq!((next.width, next.height), (8.0, 4.0));
                assert_eq!(
                    if right { next.x } else { next.x + next.width },
                    if right { p.x } else { p.x + p.width }
                );
                assert_eq!(
                    if bottom { next.y } else { next.y + next.height },
                    if bottom { p.y } else { p.y + p.height }
                );
            }
        }
    }

    #[test]
    fn horizontal_resize_works_and_unlocked_axes_are_independent() {
        let p = initial();
        let locked = drag_placement(&p, (4.0, 0.0), Some((true, true)), true);
        assert_eq!((locked.width, locked.height), (8.0, 4.0));
        let unlocked = drag_placement(&p, (0.0, 2.0), Some((true, true)), false);
        assert_eq!((unlocked.width, unlocked.height), (4.0, 4.0));
        let shrunk = drag_placement(&p, (-20.0, -20.0), Some((true, true)), true);
        assert!(shrunk.width >= 0.01 && shrunk.height >= 0.01);
        assert_eq!(shrunk.width / shrunk.height, 2.0);
    }

    #[test]
    fn translation_preserves_sampling_and_rejects_nonfinite_input() {
        let p = initial();
        let next = drag_placement(&p, (-2.0, 3.0), None, true);
        assert_eq!(
            (next.x, next.y, next.width, next.height),
            (1.0, 8.0, 4.0, 2.0)
        );
        assert_eq!(next.sampling, p.sampling);
        assert_eq!(next.pixelation, p.pixelation);
        assert_eq!(next.resolution, p.resolution);
        assert_eq!(next.transparency, p.transparency);
        assert_eq!(drag_placement(&p, (f64::NAN, 0.0), None, false), p);
    }
}
