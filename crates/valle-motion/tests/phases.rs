//! Three-phase timing tests cover endpoints, proportional compression, hold cycles and exact progress bits.

use valle_motion::{PhaseKind, PhaseLayout, PhaseSpec, motion_context_at, phase_windows};
use valle_timeline::FrameRate;

fn fps() -> FrameRate {
    FrameRate::new(30, 1).unwrap()
}

fn spec(enter: u32, exit: u32) -> PhaseSpec {
    PhaseSpec {
        enter_frames: enter,
        exit_frames: exit,
        hold_cycle_frames: None,
    }
}

fn layout(enter: u32, exit: u32, duration: u32) -> PhaseLayout {
    phase_windows(&spec(enter, exit), duration)
}

/// Return enter, hold and exit durations for table assertions.
fn windows(enter: u32, exit: u32, duration: u32) -> (u32, u32, u32) {
    let l = layout(enter, exit, duration);
    (l.enter_frames, l.hold_frames, l.exit_frames)
}

// Phase endpoints.

/// An N-frame phase accepts 0 through N-1; out-of-range frames return None.
#[test]
fn legal_frames_are_zero_to_duration_minus_one() {
    let l = layout(4, 3, 10);
    for f in 0..10 {
        assert!(
            motion_context_at(f, &l, fps()).is_some(),
            "frame {f} must exist"
        );
    }
    assert!(
        motion_context_at(10, &l, fps()).is_none(),
        "frame D is out of clip"
    );
    assert!(motion_context_at(u32::MAX, &l, fps()).is_none());

    // An empty clip has no evaluable frames.
    let empty = layout(4, 3, 0);
    assert_eq!(
        (empty.enter_frames, empty.hold_frames, empty.exit_frames),
        (0, 0, 0)
    );
    assert!(motion_context_at(0, &empty, fps()).is_none());
}

/// The final in-window progress is (N-1)/N; clamped completion first appears in the next phase.
#[test]
fn last_frame_of_phase_is_right_open_not_one() {
    let l = layout(4, 3, 10); // enter=[0,4) hold=[4,7) exit=[7,10)

    let last_enter = motion_context_at(3, &l, fps()).unwrap();
    assert!(last_enter.enter.active);
    assert_eq!(
        last_enter.enter.progress, 0.75,
        "final enter progress must equal 3/4"
    );

    let first_hold = motion_context_at(4, &l, fps()).unwrap();
    assert!(!first_hold.enter.active);
    assert_eq!(first_hold.enter.frame, 4, "out-of-window frame clamps to N");
    assert_eq!(
        first_hold.enter.progress, 1.0,
        "enter progress stays at one after completion"
    );

    // Exit progress cannot reach one inside the half-open clip interval.
    let last_frame = motion_context_at(9, &l, fps()).unwrap();
    assert!(last_frame.exit.active);
    assert_eq!(last_frame.exit.progress, 2.0 / 3.0);
    assert_eq!(last_frame.current_phase, PhaseKind::Exit);
}

/// Progress advances uniformly across the enter-to-hold boundary.
#[test]
fn enter_to_hold_has_uniform_velocity() {
    let e = 5u32;
    let l = layout(e, 0, 12);
    // Include the first frame beyond the enter window in the arithmetic progression.
    for f in 0..=e {
        let ctx = motion_context_at(f, &l, fps()).unwrap();
        assert_eq!(
            ctx.enter.progress,
            f64::from(f) / f64::from(e),
            "frame {f}: progress must equal frame/N, including out-of-window clamping"
        );
    }
    // Every step, including the boundary step, is 1/e.
    let step = 1.0 / f64::from(e);
    for f in 0..e {
        let a = motion_context_at(f, &l, fps()).unwrap().enter.progress;
        let b = motion_context_at(f + 1, &l, fps()).unwrap().enter.progress;
        assert!(
            (b - a - step).abs() < 1e-12,
            "unexpected progress step from frame {f} to {}",
            f + 1
        );
    }
}

/// One-frame and zero-length phase behavior.
#[test]
fn single_frame_and_zero_length_phases() {
    // The sole frame of a one-frame enter window has progress zero.
    let l = layout(1, 0, 3);
    let f0 = motion_context_at(0, &l, fps()).unwrap();
    assert_eq!(f0.current_phase, PhaseKind::Enter);
    assert!(f0.enter.active);
    assert_eq!(f0.enter.progress, 0.0);
    // The next frame enters hold with enter complete.
    assert_eq!(
        motion_context_at(1, &l, fps()).unwrap().current_phase,
        PhaseKind::Hold
    );
    assert_eq!(motion_context_at(1, &l, fps()).unwrap().enter.progress, 1.0);

    // A zero-length enter phase is already complete.
    let no_enter = layout(0, 2, 6);
    let g0 = motion_context_at(0, &no_enter, fps()).unwrap();
    assert_eq!(g0.current_phase, PhaseKind::Hold);
    assert!(!g0.enter.active);
    assert_eq!(g0.enter.duration_frames, 0);
    assert_eq!(g0.enter.progress, 1.0);

    // A zero-length exit phase never starts before the clip ends.
    let no_exit = layout(2, 0, 6);
    let h5 = motion_context_at(5, &no_exit, fps()).unwrap();
    assert_eq!(h5.current_phase, PhaseKind::Hold);
    assert!(!h5.exit.active);
    assert_eq!(h5.exit.progress, 0.0);

    // Compress to one enter frame and zero exit frames using half-up rounding.
    let squeezed = layout(10, 10, 1);
    assert_eq!(
        (
            squeezed.enter_frames,
            squeezed.hold_frames,
            squeezed.exit_frames
        ),
        (1, 0, 0)
    );
    let s0 = motion_context_at(0, &squeezed, fps()).unwrap();
    assert_eq!(s0.current_phase, PhaseKind::Enter);
    assert_eq!(s0.enter.progress, 0.0);
    assert_eq!(s0.exit.progress, 0.0, "zero-length exit never starts");
}

// Proportional compression.

/// Cover sufficient duration, exact boundaries, compression, degenerate inputs and rounding ties.
#[test]
fn phase_windows_table() {
    // Keep declared enter/exit durations and allocate the remainder to hold.
    assert_eq!(windows(6, 6, 30), (6, 18, 6));
    // An exact fit gives zero hold duration without compression.
    assert_eq!(windows(6, 6, 12), (6, 0, 6));
    // D < E+X：k = D/(E+X) = 0.5。
    assert_eq!(windows(6, 6, 6), (3, 0, 3));
    // Round enter first and assign the remainder to exit, preserving complete clip coverage.
    assert_eq!(windows(1, 2, 2), (1, 0, 1)); // round(1·2/3)=round(0.67)=1
    assert_eq!(windows(2, 1, 1), (1, 0, 0)); // round(2·1/3)=round(0.67)=1
    assert_eq!(windows(1, 1, 1), (1, 0, 0)); // tie：round(0.5)=1（half-away-from-zero）
    assert_eq!(windows(3, 1, 1), (1, 0, 0)); // round(3·1/4)=round(0.75)=1
    assert_eq!(windows(1, 3, 1), (0, 0, 1)); // Rounding may assign the full frame to exit; zero exit duration assigns the clip to enter.
    assert_eq!(windows(10, 0, 4), (4, 0, 0));
    // Zero enter and exit durations assign the entire clip to hold.
    assert_eq!(windows(0, 0, 7), (0, 7, 0));
    // Empty clip.
    assert_eq!(windows(6, 6, 0), (0, 0, 0));
    // A one-frame hold-only clip.
    assert_eq!(windows(0, 0, 1), (0, 1, 0));
}

/// Use u128 intermediates when doubled duration products exceed u64.
#[test]
fn phase_windows_large_frame_counts() {
    let m = u32::MAX;
    let d = m - 1;
    // E = X = u32::MAX，D = u32::MAX−1 ⇒ enter = round((M−1)/2) = 2147483647（tie half-up）。
    assert_eq!(windows(m, m, d), (2_147_483_647, 0, 2_147_483_647));
    let l = layout(m, m, d);
    assert_eq!(
        l.enter_frames + l.hold_frames + l.exit_frames,
        d,
        "cover [0, D) exactly"
    );
}

/// Phases must be mutually exclusive and cover [0, D).
#[test]
fn phases_are_exhaustive_and_mutually_exclusive() {
    for e in 0..8u32 {
        for x in 0..8u32 {
            for d in 0..20u32 {
                let l = layout(e, x, d);
                assert_eq!(
                    l.enter_frames + l.hold_frames + l.exit_frames,
                    d,
                    "E={e} X={x} D={d}"
                );
                for f in 0..d {
                    let ctx = motion_context_at(f, &l, fps()).unwrap();
                    // Exactly one phase is active, matching current_phase.
                    let actives = [ctx.enter.active, ctx.hold.active, ctx.exit.active];
                    assert_eq!(
                        actives.iter().filter(|a| **a).count(),
                        1,
                        "E={e} X={x} D={d} f={f}: phases must be mutually exclusive and cover the clip"
                    );
                    let expected = match ctx.current_phase {
                        PhaseKind::Enter => 0,
                        PhaseKind::Hold => 1,
                        PhaseKind::Exit => 2,
                    };
                    assert!(
                        actives[expected],
                        "E={e} X={x} D={d} f={f}: active flags disagree with currentPhase"
                    );
                }
            }
        }
    }
}

// Hold cycles.

#[test]
fn hold_cycle_counters() {
    let l = phase_windows(
        &PhaseSpec {
            enter_frames: 2,
            exit_frames: 2,
            hold_cycle_frames: Some(5),
        },
        20,
    );
    assert_eq!((l.enter_frames, l.hold_frames, l.exit_frames), (2, 16, 2));

    // The hold window is [2, 18), with local frame equal to frame minus two.
    let at = |frame: u32| motion_context_at(frame, &l, fps()).unwrap().hold;
    let h0 = at(2);
    assert_eq!(
        (h0.iteration, h0.cycle_frame, h0.cycle_progress),
        (0, 0, 0.0)
    );
    let h4 = at(6); // The cycle's last frame has progress 4/5.
    assert_eq!(
        (h4.iteration, h4.cycle_frame, h4.cycle_progress),
        (0, 4, 0.8)
    );
    let h5 = at(7); // Start the next cycle.
    assert_eq!(
        (h5.iteration, h5.cycle_frame, h5.cycle_progress),
        (1, 0, 0.0)
    );
    let h15 = at(17); // Three full cycles followed by the first frame of cycle four.
    assert_eq!(
        (h15.iteration, h15.cycle_frame, h15.cycle_progress),
        (3, 0, 0.0)
    );

    // Track whole-phase and per-cycle progress independently.
    let mid = at(10); // f = 8
    assert_eq!(mid.progress, 8.0 / 16.0);
    assert_eq!(mid.cycle_progress, 3.0 / 5.0);

    // Before hold starts, all hold counters are zero.
    let before = at(0);
    assert!(!before.active);
    assert_eq!(
        (before.frame, before.iteration, before.cycle_frame),
        (0, 0, 0)
    );
    assert_eq!((before.progress, before.cycle_progress), (0.0, 0.0));
    // After hold, clamp to the closed endpoint of the final partial cycle.
    let after = at(19);
    assert!(!after.active);
    assert_eq!(after.frame, 16);
    assert_eq!(after.progress, 1.0);
    assert_eq!(
        (after.iteration, after.cycle_frame, after.cycle_progress),
        (3, 1, 0.2)
    );
}

/// When cycles divide hold duration exactly, exit reads the final cycle's closed endpoint rather than an unplayed next cycle.
#[test]
fn hold_cycle_exit_reads_last_cycle_closed_end_when_divisible() {
    // A ten-frame hold contains two complete five-frame cycles.
    let l = phase_windows(
        &PhaseSpec {
            enter_frames: 2,
            exit_frames: 2,
            hold_cycle_frames: Some(5),
        },
        14,
    );
    assert_eq!((l.enter_frames, l.hold_frames, l.exit_frames), (2, 10, 2));
    let at = |frame: u32| motion_context_at(frame, &l, fps()).unwrap().hold;

    // The last hold frame is frame four of cycle one, with progress 0.8.
    let last = at(11);
    assert_eq!(
        (last.iteration, last.cycle_frame, last.cycle_progress),
        (1, 4, 0.8)
    );
    // Exit retains cycle one's closed endpoint.
    for f in [12, 13] {
        let after = at(f);
        assert!(!after.active);
        assert_eq!(
            (after.iteration, after.cycle_frame, after.cycle_progress),
            (1, 5, 1.0),
            "frame {f}: exit must retain the final cycle endpoint without jumping back"
        );
    }
}

/// A missing or zero cycle duration treats hold as one cycle.
#[test]
fn hold_without_cycle_degenerates_to_single_iteration() {
    for cycle in [None, Some(0)] {
        let l = phase_windows(
            &PhaseSpec {
                enter_frames: 1,
                exit_frames: 1,
                hold_cycle_frames: cycle,
            },
            10,
        );
        for frame in 0..10 {
            let h = motion_context_at(frame, &l, fps()).unwrap().hold;
            assert_eq!(h.iteration, 0, "cycle={cycle:?} frame={frame}");
            assert_eq!(h.cycle_frame, h.frame);
            assert_eq!(h.cycle_progress, h.progress);
        }
    }
}

// Exact f64 progress bits.

/// Derive progress with one IEEE 754 division of integer frames; a seven-frame phase detects reciprocal or accumulation drift.
#[test]
fn progress_bit_patterns_are_frozen() {
    const EXPECTED: [u64; 7] = [
        0x0000_0000_0000_0000, // 0/7
        0x3FC2_4924_9249_2492, // 1/7
        0x3FD2_4924_9249_2492, // 2/7
        0x3FDB_6DB6_DB6D_B6DB, // 3/7
        0x3FE2_4924_9249_2492, // 4/7
        0x3FE6_DB6D_B6DB_6DB7, // 5/7
        0x3FEB_6DB6_DB6D_B6DB, // 6/7
    ];
    let l = layout(7, 0, 12);
    for (f, want) in EXPECTED.iter().enumerate() {
        let got = motion_context_at(f as u32, &l, fps())
            .unwrap()
            .enter
            .progress;
        assert_eq!(got.to_bits(), *want, "frame {f}: progress bits changed");
    }
    // Clamped completion must be exactly 1.0.
    assert_eq!(
        motion_context_at(7, &l, fps())
            .unwrap()
            .enter
            .progress
            .to_bits(),
        1.0f64.to_bits()
    );
    // Cycle progress uses the same integer-frame division.
    let cyc = phase_windows(
        &PhaseSpec {
            enter_frames: 0,
            exit_frames: 0,
            hold_cycle_frames: Some(6),
        },
        6,
    );
    assert_eq!(
        motion_context_at(1, &cyc, fps())
            .unwrap()
            .hold
            .cycle_progress
            .to_bits(),
        0x3FC5_5555_5555_5555, // 1/6
    );
    assert_eq!(
        motion_context_at(5, &cyc, fps())
            .unwrap()
            .hold
            .cycle_progress
            .to_bits(),
        0x3FEA_AAAA_AAAA_AAAB, // 5/6
    );
}
