// fan.rs: Fan Controller (Auto, Max, Curve)
// 7-point independent curve calculation, keep-alive loop, and >=90°C emergency max.

use crate::ec::EmbeddedController;
use crate::hwmon::HwmonBackend;
use lightomen_common::{CURVE_TEMPS, FAN_MODE_AUTO, FAN_MODE_CURVE, FAN_MODE_MAX};

pub struct FanController;

impl FanController {
    /// Piecewise linear interpolation for a 7-point fan curve
    pub fn evaluate_curve(curve: &[u8; 7], temp: u16) -> u8 {
        if temp <= CURVE_TEMPS[0] as u16 {
            return curve[0];
        }
        if temp >= CURVE_TEMPS[6] as u16 {
            return curve[6];
        }

        for i in 0..6 {
            let t0 = CURVE_TEMPS[i] as u16;
            let t1 = CURVE_TEMPS[i + 1] as u16;
            if temp >= t0 && temp <= t1 {
                let p0 = curve[i] as f32;
                let p1 = curve[i + 1] as f32;
                let fraction = (temp - t0) as f32 / (t1 - t0) as f32;
                // If starting from 0% (fan off / silent mode), stay at 0 RPM
                // until fraction crosses halfway (e.g. 30°C..34°C stays 0 RPM)
                if p0 == 0.0 && p1 > 0.0 && fraction < 0.5 {
                    return 0;
                }
                let result = p0 + fraction * (p1 - p0);
                return (result.round() as u8).clamp(0, 100);
            }
        }

        curve[6]
    }

    /// Convert fan percentage (0..100) to target RPM
    pub fn pct_to_rpm(pct: u8, max_rpm: u16) -> u16 {
        if pct == 0 {
            0
        } else {
            let rpm = (pct as u32 * max_rpm as u32 / 100) as u16;
            // Floor of functional spinning speed is 1100 RPM on HP Victus (8BD4) and OMEN
            rpm.clamp(1100, max_rpm)
        }
    }

    /// Apply fan mode and calculated speeds to hardware
    pub fn apply(
        ec: &EmbeddedController,
        hwmon: &mut HwmonBackend,
        mode: u8,
        cpu_curve: &[u8; 7],
        gpu_curve: &[u8; 7],
        cpu_temp: u16,
        gpu_temp: u16,
        gpu_zero_rpm_enabled: u8,
        on_battery: bool,
    ) {
        let max_rpm = hwmon.max_rpm;
        let was_max = hwmon.last_applied_mode == Some(FAN_MODE_MAX);

        // EMERGENCY MAX OVERRIDE: CPU >= 95°C or GPU >= 90°C
        if cpu_temp >= 95 || gpu_temp >= 90 {
            hwmon.apply_fan_mode(FAN_MODE_MAX, max_rpm, max_rpm);
            return;
        }

        // Direct EC hardware unlatch when leaving MAX fan mode
        if was_max && mode != FAN_MODE_MAX {
            ec.unlatch_max_fan(1100);
        }

        match mode {
            FAN_MODE_AUTO => {
                hwmon.apply_fan_mode(FAN_MODE_AUTO, 0, 0);
            }
            FAN_MODE_MAX => {
                hwmon.apply_fan_mode(FAN_MODE_MAX, max_rpm, max_rpm);
            }
            FAN_MODE_CURVE => {
                let cpu_pct = Self::evaluate_curve(cpu_curve, cpu_temp);
                let target1 = Self::pct_to_rpm(cpu_pct, max_rpm);

                let target2 = if gpu_curve == cpu_curve {
                    // Linked curves (CPU + GPU linked together):
                    // Both fans must spin at the exact same RPM!
                    if on_battery && gpu_zero_rpm_enabled == 1 && gpu_temp == 0 && cpu_temp < 75 {
                        0
                    } else {
                        target1
                    }
                } else if gpu_temp == 0 {
                    // NVIDIA dGPU is in D3cold power-saving sleep (Independent curves)
                    if target1 == 0 {
                        0
                    } else if on_battery && gpu_zero_rpm_enabled == 1 {
                        if cpu_temp >= 75 {
                            target1
                        } else {
                            0
                        }
                    } else {
                        let gpu_pct = Self::evaluate_curve(gpu_curve, 35);
                        Self::pct_to_rpm(gpu_pct, max_rpm).min(target1)
                    }
                } else {
                    let gpu_pct = Self::evaluate_curve(gpu_curve, gpu_temp);
                    Self::pct_to_rpm(gpu_pct, max_rpm)
                };

                hwmon.apply_fan_mode(FAN_MODE_CURVE, target1, target2);
            }
            _ => {
                hwmon.apply_fan_mode(FAN_MODE_AUTO, 0, 0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_evaluate_curve_endpoints() {
        let curve = [20, 30, 40, 50, 60, 70, 100];
        assert_eq!(FanController::evaluate_curve(&curve, 25), 20);
        assert_eq!(FanController::evaluate_curve(&curve, 30), 20);
        assert_eq!(FanController::evaluate_curve(&curve, 90), 100);
        assert_eq!(FanController::evaluate_curve(&curve, 95), 100);
    }

    #[test]
    fn test_evaluate_curve_midpoint() {
        let curve = [20, 30, 40, 50, 60, 70, 100];
        // 35°C is midway between 30 (20%) and 40 (30%) -> 25%
        assert_eq!(FanController::evaluate_curve(&curve, 35), 25);
    }

    #[test]
    fn test_evaluate_curve_zero_rpm() {
        let curve = [0, 20, 30, 40, 50, 60, 100];
        assert_eq!(FanController::evaluate_curve(&curve, 25), 0);
        assert_eq!(FanController::evaluate_curve(&curve, 29), 0);
        assert_eq!(FanController::evaluate_curve(&curve, 30), 0);
        assert_eq!(FanController::evaluate_curve(&curve, 32), 0);
        assert_eq!(FanController::evaluate_curve(&curve, 34), 0);
        assert_eq!(FanController::evaluate_curve(&curve, 35), 10);
        assert_eq!(FanController::evaluate_curve(&curve, 40), 20);
    }

    #[test]
    fn test_pct_to_rpm() {
        let max = 5800;
        assert_eq!(FanController::pct_to_rpm(0, max), 0);
        assert_eq!(FanController::pct_to_rpm(100, max), 5800);
        assert_eq!(FanController::pct_to_rpm(50, max), 2900);
        // Minimum functional floor clamps < 1100 to 1100 when pct > 0
        assert_eq!(FanController::pct_to_rpm(10, max), 1100);
        assert_eq!(FanController::pct_to_rpm(18, max), 1100);
        assert_eq!(FanController::pct_to_rpm(20, max), 1160);
    }

    #[test]
    fn test_zero_rpm_to_spinup_progression() {
        let curve = [0, 20, 30, 40, 50, 60, 100];
        let max_rpm = 5800;

        // At 29°C (cold boot): 0% -> 0 RPM
        let pct_29 = FanController::evaluate_curve(&curve, 29);
        assert_eq!(pct_29, 0);
        assert_eq!(FanController::pct_to_rpm(pct_29, max_rpm), 0);

        // At 35°C: 10% -> 1100 RPM (quiet physical floor)
        let pct_35 = FanController::evaluate_curve(&curve, 35);
        assert_eq!(pct_35, 10);
        assert_eq!(FanController::pct_to_rpm(pct_35, max_rpm), 1100);

        // At 40°C: 20% -> 1160 RPM
        let pct_40 = FanController::evaluate_curve(&curve, 40);
        assert_eq!(pct_40, 20);
        assert_eq!(FanController::pct_to_rpm(pct_40, max_rpm), 1160);

        // At 60°C: 40% -> 2320 RPM
        let pct_60 = FanController::evaluate_curve(&curve, 60);
        assert_eq!(pct_60, 40);
        assert_eq!(FanController::pct_to_rpm(pct_60, max_rpm), 2320);

        // At 90°C: 100% -> 5800 RPM
        let pct_90 = FanController::evaluate_curve(&curve, 90);
        assert_eq!(pct_90, 100);
        assert_eq!(FanController::pct_to_rpm(pct_90, max_rpm), 5800);
    }

    #[test]
    fn test_evaluate_curve_monotonic() {
        let curve = [0, 20, 30, 40, 50, 60, 100];
        let mut prev = 0;
        for temp in 20..=105 {
            let pct = FanController::evaluate_curve(&curve, temp);
            assert!(pct >= prev, "Curve decreased at temp {}: {} < {}", temp, pct, prev);
            prev = pct;
        }
    }

    #[test]
    fn test_pct_to_rpm_various_max() {
        for max_rpm in [4500, 5000, 5800, 6000, 6400] {
            assert_eq!(FanController::pct_to_rpm(0, max_rpm), 0);
            assert_eq!(FanController::pct_to_rpm(1, max_rpm), 1100);
            assert_eq!(FanController::pct_to_rpm(100, max_rpm), max_rpm);
            let mid = FanController::pct_to_rpm(50, max_rpm);
            assert_eq!(mid, max_rpm / 2);
        }
    }
}

#[cfg(test)]
mod curve_tests {
    use super::FanController;
    use lightomen_common::{CURVE_TEMPS, DEFAULT_CPU_CURVE, DEFAULT_GPU_CURVE};

    #[test]
    fn curve_hits_every_control_point() {
        for curve in [DEFAULT_CPU_CURVE, DEFAULT_GPU_CURVE] {
            for (i, &t) in CURVE_TEMPS.iter().enumerate() {
                assert_eq!(FanController::evaluate_curve(&curve, t as u16), curve[i]);
            }
        }
    }

    #[test]
    fn curve_stays_between_neighbouring_points() {
        let curve = DEFAULT_CPU_CURVE;
        for i in 0..6 {
            let (lo, hi) = (curve[i].min(curve[i + 1]), curve[i].max(curve[i + 1]));
            for t in CURVE_TEMPS[i]..=CURVE_TEMPS[i + 1] {
                let pct = FanController::evaluate_curve(&curve, t as u16);
                assert!(pct >= lo && pct <= hi, "{}C -> {}%", t, pct);
            }
        }
    }

    #[test]
    fn curve_handles_descending_segment() {
        let curve = [50, 40, 40, 40, 40, 40, 40];
        assert_eq!(FanController::evaluate_curve(&curve, 35), 45);
    }

    #[test]
    fn flat_zero_curve_stays_off() {
        let curve = [0; 7];
        for t in 0..=120 {
            assert_eq!(FanController::evaluate_curve(&curve, t), 0);
        }
        assert_eq!(FanController::pct_to_rpm(0, 5800), 0);
    }

    #[test]
    fn pct_to_rpm_is_monotonic() {
        let mut prev = 0;
        for pct in 0..=100 {
            let rpm = FanController::pct_to_rpm(pct, 5800);
            assert!(rpm >= prev);
            prev = rpm;
        }
    }
}
