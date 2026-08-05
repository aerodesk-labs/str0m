use crate::rtp_::Bitrate;

const PACING_FACTOR: f64 = 1.1;

/// Target padding rate when media is active. This maintains NAT bindings, RTX state,
/// and allows ALR periodic probes to discover higher bandwidth.
const PADDING_TARGET: Bitrate = Bitrate::bps(50_000);

pub(crate) struct PacingResult {
    /// The bitrate at which the pacer may emit padding **when there is no media queued**.
    ///
    /// This value is intentionally an *absolute* target used by the pacer, not a “delta to add
    /// on top of current media”. In practice the **effective padding** over time is the
    /// difference between this target and the media actually sent, because padding is only used
    /// to fill gaps (empty queues), not to continuously top-up while media is flowing.
    pub padding_rate: Bitrate,
    pub pacing_rate: Bitrate,
}

/// Controls the pacing and padding rates.
pub(crate) struct PacerControl {}

impl PacerControl {
    pub fn new() -> Self {
        Self {}
    }

    pub fn calculate(
        &self,
        current_bitrate: Option<Bitrate>,
        has_active_media: bool,
        estimate: Bitrate,
        is_overuse: bool,
    ) -> PacingResult {
        let padding_enabled = current_bitrate.map_or(has_active_media, |v| !v.is_zero());
        let padding_rate = if is_overuse || !padding_enabled {
            Bitrate::ZERO
        } else {
            PADDING_TARGET.min(estimate)
        };

        let min_pacing_rate = current_bitrate.unwrap_or(estimate).max(estimate) * PACING_FACTOR;
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

        let r = c.calculate(None, true, Bitrate::kbps(1_000), false);

        assert_eq!(r.padding_rate, PADDING_TARGET);
    }

    #[test]
    fn no_padding_without_active_media() {
        let c = PacerControl::new();

        let r = c.calculate(None, false, Bitrate::kbps(1_000), false);

        assert_eq!(r.padding_rate, Bitrate::ZERO);
    }

    #[test]
    fn overuse_suppresses_padding() {
        let c = PacerControl::new();

        let r = c.calculate(Some(Bitrate::kbps(500)), true, Bitrate::mbps(40), true);

        assert_eq!(r.padding_rate, Bitrate::ZERO);
    }

    #[test]
    fn current_bitrate_acts_as_pacing_floor() {
        let c = PacerControl::new();

        let r = c.calculate(Some(Bitrate::kbps(2_000)), true, Bitrate::kbps(500), false);

        assert_eq!(r.pacing_rate, Bitrate::kbps(2_000) * PACING_FACTOR);
    }

    #[test]
    fn estimate_caps_padding() {
        let c = PacerControl::new();
        let estimate = Bitrate::kbps(20);

        let r = c.calculate(Some(Bitrate::kbps(500)), true, estimate, false);

        assert_eq!(r.padding_rate, estimate);
    }

    #[test]
    fn explicit_zero_allocation_disables_padding() {
        let c = PacerControl::new();

        let r = c.calculate(Some(Bitrate::ZERO), true, Bitrate::kbps(1_000), false);

        assert_eq!(r.padding_rate, Bitrate::ZERO);
    }
}
