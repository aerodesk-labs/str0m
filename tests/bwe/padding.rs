//! BWE tests for how much padding the pacer emits.
//!
//! str0m sends padding as spurious RTX resends (`StreamTx::poll_packet_padding`), so an
//! oversized padding target shows up on the wire - and in the remote's
//! `retransmittedBytesReceived` - as a flood of retransmissions. These tests pin the padding
//! volume down by counting bytes sent on the RTX SSRC.

use std::time::Duration;

use netem::{DataSize, NetemConfig};
use str0m::RtcError;
use str0m::bwe::Bitrate;

use crate::common::{BweTestContext, Step, connect_with_bwe, init_crypto_default, init_log};

/// SSRCs declared by [`BweTestContext::new`].
const SSRC_MEDIA: u32 = 42;
const SSRC_RTX: u32 = 44;

/// Padding must not scale with the bandwidth estimate.
///
/// With media far below both the estimate and `desired_bitrate`, an estimate-derived padding
/// target fills the whole gap with RTX. libWebRTC caps padding at `max_padding_rate`, which is
/// derived from the configured layers rather than from the estimate.
///
/// Regression test for the padding target being `estimate.min(desired_bitrate)`, which sent
/// RTX at roughly the media bitrate itself.
#[test]
pub fn padding_does_not_scale_with_estimate() -> Result<(), RtcError> {
    init_log();
    init_crypto_default();

    // Media at 500 kbps against a 5 Mbps link with 5 Mbps desired: a wide gap for padding to
    // fill if it is allowed to track the estimate.
    let plan = vec![
        Step::Conditions {
            description: "Roomy network",
            config: NetemConfig::new()
                .link(Bitrate::mbps(5), DataSize::kbytes(100))
                .seed(42),
        },
        Step::Media {
            description: "Ramp up so the estimate climbs",
            desired_bitrate: Bitrate::mbps(5),
            media_send_rate: Bitrate::mbps(2),
        },
        Step::Run {
            description: "Let initial probing settle",
            duration: Duration::from_secs(3),
        },
        Step::Media {
            description: "Drop media well below the estimate",
            desired_bitrate: Bitrate::mbps(5),
            media_send_rate: Bitrate::kbps(500),
        },
        Step::Run {
            description: "Steady state with a large media/estimate gap",
            duration: Duration::from_secs(5),
        },
    ];

    let (mut l, mut r) = connect_with_bwe(Bitrate::mbps(2), Bitrate::mbps(5));
    let mut ctx = BweTestContext::new(&mut l, &mut r);

    // The application has 500 kbps allocated to media.
    l.bwe().set_current_bitrate(Bitrate::kbps(500));

    ctx.run_plan(&mut l, &mut r, &plan)?;

    let media = l.sent_bytes_by_ssrc.get(&SSRC_MEDIA).copied().unwrap_or(0);
    let rtx = l.sent_bytes_by_ssrc.get(&SSRC_RTX).copied().unwrap_or(0);

    assert!(media > 0, "expected media to have been sent");

    // Probe clusters are also carried on the RTX SSRC, so some RTX is expected. What must not
    // happen is RTX approaching the media volume, which is what an estimate-derived padding
    // target produced (~1:1 with media).
    // On the estimate-derived target this ratio was 2.6, i.e. more RTX than media.
    let ratio = rtx as f64 / media as f64;
    assert!(
        ratio < 0.5,
        "RTX bytes should stay well below media bytes, got {rtx} RTX vs {media} media \
         (ratio {ratio:.2}). Padding is likely tracking the estimate again."
    );

    Ok(())
}

/// Zero allocated bitrate disables padding entirely.
///
/// libWebRTC's `max_padding_rate` defaults to zero, and a sender with nothing allocated pads
/// nothing.
#[test]
pub fn no_padding_without_allocated_media() -> Result<(), RtcError> {
    init_log();
    init_crypto_default();

    let plan = vec![
        Step::Conditions {
            description: "Roomy network",
            config: NetemConfig::new()
                .link(Bitrate::mbps(5), DataSize::kbytes(100))
                .seed(42),
        },
        Step::Media {
            description: "Send media without declaring an allocation",
            desired_bitrate: Bitrate::mbps(5),
            media_send_rate: Bitrate::kbps(500),
        },
        Step::Run {
            description: "Steady state",
            duration: Duration::from_secs(5),
        },
    ];

    let (mut l, mut r) = connect_with_bwe(Bitrate::mbps(2), Bitrate::mbps(5));
    let mut ctx = BweTestContext::new(&mut l, &mut r);

    // Deliberately leave current_bitrate at zero.
    ctx.run_plan(&mut l, &mut r, &plan)?;

    let media = l.sent_bytes_by_ssrc.get(&SSRC_MEDIA).copied().unwrap_or(0);
    let rtx = l.sent_bytes_by_ssrc.get(&SSRC_RTX).copied().unwrap_or(0);

    assert!(media > 0, "expected media to have been sent");

    // Probes still use the RTX SSRC; only continuous padding is gated off here.
    let ratio = rtx as f64 / media as f64;
    assert!(
        ratio < 0.5,
        "no padding should be sent without an allocation, got {rtx} RTX vs {media} media \
         (ratio {ratio:.2})"
    );

    Ok(())
}
