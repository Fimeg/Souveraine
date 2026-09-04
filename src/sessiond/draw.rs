//! Software renderer for sessiond's holding surface.
//!
//! A dark field. That is the whole surface, deliberately.
//!
//! Until 2026-07-29 this drew a PIN pad — dots, a 3x4 keypad, a 5x7 digit
//! font, a failed-attempt counter — because sessiond could authenticate and
//! unlock. It cannot any more, and it should not: the protocol has a lock
//! directive and no unlock directive, so a successful PAM here opened the
//! compositor's lock and had no way to tell the shell, which then re-locked on
//! its next registration. A keypad that takes input and cannot finish the job
//! is worse than no keypad, so the keypad is gone rather than left inert.
//!
//! Nothing replaced it. A glyph on a locked screen with no text renderer is
//! not legible — the honest place for "the shell is not running and this
//! session is held locked" is the journal, where it can be read, searched and
//! correlated. `lock.rs` says it there.
//!
//! All drawing targets a raw ARGB8888 buffer (little-endian: B,G,R,A).

/// The field. Matches the shell lockscreen's backdrop closely enough that a
/// handoff in either direction is not a visible flash.
pub const BG: u32 = 0xFF0E0E12;

/// What the surface is currently expressing. One variant today; kept as an
/// enum because the surface still has states worth distinguishing later
/// (holding for a shell that is starting, vs holding over a dead one) and a
/// bool would have to be renamed to say so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mood {
    /// The session is locked and this daemon is holding it.
    Holding,
}

#[derive(Debug, Clone, Copy)]
pub struct Scene {
    pub mood: Mood,
}

pub fn render(buf: &mut [u32], w: i32, h: i32, scene: &Scene) {
    debug_assert_eq!(buf.len(), (w * h) as usize);
    let _ = scene;
    let _ = (w, h);
    buf.fill(BG);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_fills_the_whole_buffer_with_the_field() {
        let (w, h) = (8, 4);
        let mut buf = vec![0u32; (w * h) as usize];
        render(
            &mut buf,
            w,
            h,
            &Scene {
                mood: Mood::Holding,
            },
        );
        assert!(buf.iter().all(|&px| px == BG));
    }

    #[test]
    fn render_is_opaque_everywhere() {
        // A lock surface that is even partly transparent shows the session
        // underneath it, which is the one thing it exists to prevent.
        let (w, h) = (4, 4);
        let mut buf = vec![0u32; (w * h) as usize];
        render(
            &mut buf,
            w,
            h,
            &Scene {
                mood: Mood::Holding,
            },
        );
        assert!(buf.iter().all(|&px| px >> 24 == 0xFF));
    }
}
