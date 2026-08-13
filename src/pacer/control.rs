use crate::rtp_::Bitrate;

const PACING_FACTOR: f64 = 1.1;

/// Target padding rate when media is active.
///
/// This mirrors libWebRTC's `max_padding_rate`, which is *not* derived from the bandwidth
/// estimate. It is computed by `CalculateMaxPadBitrateBps()` in `video_send_stream_impl.cc`
/// from the configured encoder layers. With ALR probing enabled - which str0m always does -
/// that collapses to the min bitrate of the lowest simulcast layer:
///
/// > With alr probing, just pad to the min bitrate of the lowest stream, probing will handle
/// > the rest of the rampup.
///
/// So padding exists only to maintain NAT bindings and RTX state. Discovering additional
/// capacity is the job of the periodic ALR probes, not of continuous padding.
const PADDING_TARGET: Bitrate = Bitrate::bps(50_000);

pub(crate) struct PacingResult {
    pub padding_rate: Bitrate,
    pub pacing_rate: Bitrate,
}

/// Controls the pacing and padding rates.
///
/// This follows libWebRTC's `GoogCcNetworkController::GetPacingRates()`.
pub(crate) struct PacerControl {}

impl PacerControl {
    pub fn new() -> Self {
        Self {}
    }

    /// Calculate pacing and padding rates.
    ///
    /// `current_bitrate` is the bitrate currently allocated to media. Padding is only sent
    /// when media is actually allocated.
    ///
    /// Note the estimate only ever *caps* the padding rate, matching libWebRTC's
    /// `padding_rate = std::min(padding_rate, last_pushback_target_rate_)`.
    pub fn calculate(
        &self,
        current_bitrate: Bitrate,
        estimate: Bitrate,
        is_overuse: bool,
    ) -> PacingResult {
        // libWebRTC suppresses padding while congested, see `PacingController::PaddingToAdd()`:
        // `if (congested_) { return DataSize::Zero(); }`. Padding during overuse would only
        // re-excite a network we already know is struggling.
        let padding_rate = if is_overuse || current_bitrate.is_zero() {
            Bitrate::ZERO
        } else {
            PADDING_TARGET.min(estimate)
        };

        // Set pacing rate to smooth out media transmission (burst avoidance). libWebRTC uses
        // `max(min_total_allocated_bitrate_, target) * pacing_factor_`, so an allocation above
        // the estimate raises the pacing floor rather than queueing behind it. Also kept high
        // enough to allow the padding we want to send.
        let min_pacing_rate = current_bitrate.max(estimate) * PACING_FACTOR;
        let pacing_rate = min_pacing_rate.max(padding_rate);

        PacingResult {
            padding_rate,
            pacing_rate,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn padding_enabled_with_active_media() {
        let c = PacerControl::new();

        let r = c.calculate(Bitrate::kbps(500), Bitrate::kbps(1_000), false);

        assert_eq!(r.padding_rate, PADDING_TARGET);
    }

    #[test]
    fn no_padding_without_active_media() {
        let c = PacerControl::new();

        let r = c.calculate(Bitrate::ZERO, Bitrate::kbps(1_000), false);

        assert_eq!(r.padding_rate, Bitrate::ZERO);
    }

    /// The regression this change is about: padding must not scale with the estimate, or the
    /// gap between media and the estimate gets filled with spurious RTX resends.
    #[test]
    fn padding_does_not_scale_with_estimate() {
        let c = PacerControl::new();

        let low = c.calculate(Bitrate::kbps(500), Bitrate::kbps(800), false);
        let high = c.calculate(Bitrate::kbps(500), Bitrate::mbps(10), false);

        assert_eq!(low.padding_rate, PADDING_TARGET);
        assert_eq!(high.padding_rate, PADDING_TARGET);
    }

    #[test]
    fn overuse_suppresses_padding() {
        let c = PacerControl::new();

        let r = c.calculate(Bitrate::kbps(500), Bitrate::mbps(40), true);

        assert_eq!(r.padding_rate, Bitrate::ZERO);
    }

    /// `current_bitrate` is libWebRTC's `min_total_allocated_bitrate` in
    /// `max(min_total_allocated_bitrate_, target) * pacing_factor_`.
    #[test]
    fn current_bitrate_acts_as_pacing_floor() {
        let c = PacerControl::new();

        let r = c.calculate(Bitrate::kbps(2_000), Bitrate::kbps(500), false);

        assert_eq!(r.pacing_rate, Bitrate::kbps(2_000) * PACING_FACTOR);
    }

    #[test]
    fn estimate_drives_pacing_when_above_allocation() {
        let c = PacerControl::new();

        let r = c.calculate(Bitrate::kbps(500), Bitrate::kbps(2_000), false);

        assert_eq!(r.pacing_rate, Bitrate::kbps(2_000) * PACING_FACTOR);
    }

    /// A very low estimate caps padding, mirroring the `last_pushback_target_rate_` clamp.
    #[test]
    fn estimate_caps_padding() {
        let c = PacerControl::new();
        let estimate = Bitrate::kbps(20);

        let r = c.calculate(Bitrate::kbps(500), estimate, false);

        assert_eq!(r.padding_rate, estimate);
    }
}
