//! Daily demand forecast per product (FR-6, FR-7, FR-8).
//!
//! **Local stand-in for BigQuery `AI.FORECAST` (TimesFM).** The production
//! design runs TimesFM as a scheduled query; this module produces the same
//! output — a point forecast and an upper prediction bound per day — from a
//! weekday-seasonal level model, so the reorder and expiry rules above it are
//! the real thing and only the model behind them changes.
//!
//! Rules that hold for both models:
//! - stockout and closed days are excluded from fitting (censored demand, FR-3);
//! - closures the owner announced forecast zero (FR-31);
//! - short series fall back to a plain moving average and say `Low` (FR-7);
//! - accuracy is measured on a 7-day holdout as WAPE, next to a 4-week moving
//!   average baseline, so the UI can admit when the model is no better (FR-8).

use chrono::{Datelike, Duration, NaiveDate};

use crate::shop::DayDemand;

/// z for a one-sided 80% bound: the "upper prediction interval" the PRD uses
/// for safety stock.
const Z80: f64 = 1.2816;
/// Days of usable history used to fit the level and weekday shape.
const WINDOW: usize = 28;
/// Below this many usable days the series is "cold start".
const MIN_USABLE: usize = 14;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    High,
    Low,
}

impl Confidence {
    pub fn label(self) -> &'static str {
        match self {
            Confidence::High => "High confidence",
            Confidence::Low => "Low confidence",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Forecast {
    /// First day forecast (today).
    pub start: NaiveDate,
    /// Point forecast per day from `start`.
    pub mean: Vec<f64>,
    /// Upper bound of the 80% interval per day.
    pub upper: Vec<f64>,
    pub confidence: Confidence,
    pub method: &'static str,
}

impl Forecast {
    /// Sum of the point forecast over `[from, from + days)`, in days from
    /// `start`. Days past the horizon repeat the last week.
    pub fn demand(&self, from: usize, days: usize) -> f64 {
        (from..from + days).map(|i| self.mean_at(i)).sum()
    }

    pub fn mean_at(&self, i: usize) -> f64 {
        at(&self.mean, i)
    }

    pub fn upper_at(&self, i: usize) -> f64 {
        at(&self.upper, i)
    }

    /// Average daily demand over the first week, on days the shop is open:
    /// a closure is not a slow day, so it must not drag the rate quoted to
    /// the owner down.
    pub fn daily_rate(&self) -> f64 {
        let open: Vec<f64> = (0..7)
            .map(|i| self.mean_at(i))
            .filter(|m| *m > 0.0)
            .collect();
        if open.is_empty() {
            0.0
        } else {
            open.iter().sum::<f64>() / open.len() as f64
        }
    }
}

fn at(v: &[f64], i: usize) -> f64 {
    match v.len() {
        0 => 0.0,
        n if i < n => v[i],
        // Repeat the last full week: weekday shape is preserved.
        n if n >= 7 => v[n - 7 + (i - n) % 7],
        n => v[n - 1],
    }
}

fn usable(history: &[DayDemand]) -> Vec<&DayDemand> {
    history
        .iter()
        .filter(|d| !d.stockout && !d.closed)
        .collect()
}

pub fn forecast(
    history: &[DayDemand],
    start: NaiveDate,
    horizon: usize,
    closures: &[NaiveDate],
) -> Forecast {
    let usable = usable(history);
    let recent: Vec<&DayDemand> = usable.iter().rev().take(WINDOW).rev().copied().collect();

    let dates = (0..horizon).map(|i| start + Duration::days(i as i64));

    if recent.len() < MIN_USABLE {
        // Cold start: a flat moving average, with a wide band to match.
        let level = mean(recent.iter().map(|d| d.qty));
        let band = level * 0.5;
        let (mean, upper) = dates
            .map(|d| {
                if closures.contains(&d) {
                    (0.0, 0.0)
                } else {
                    (level, level + band)
                }
            })
            .unzip();
        return Forecast {
            start,
            mean,
            upper,
            confidence: Confidence::Low,
            method: "moving average (short history)",
        };
    }

    let level = mean(recent.iter().map(|d| d.qty));
    let factors = weekday_factors(&recent, level);
    let fitted = |d: NaiveDate| level * factors[d.weekday().num_days_from_monday() as usize];

    // Residual spread of the fit, for the prediction interval.
    let sigma = std_dev(recent.iter().map(|d| d.qty - fitted(d.date)));
    let cv = if level > 0.0 { sigma / level } else { 1.0 };

    let (mean, upper) = dates
        .map(|d| {
            if closures.contains(&d) {
                (0.0, 0.0)
            } else {
                let m = fitted(d);
                (m, m + Z80 * sigma)
            }
        })
        .unzip();

    Forecast {
        start,
        mean,
        upper,
        // Very noisy series get the same honest label as short ones.
        confidence: if cv <= 0.6 {
            Confidence::High
        } else {
            Confidence::Low
        },
        method: "weekday-seasonal level (TimesFM stand-in)",
    }
}

fn weekday_factors(days: &[&DayDemand], level: f64) -> [f64; 7] {
    let mut f = [1.0; 7];
    if level <= 0.0 {
        return f;
    }
    for (wd, slot) in f.iter_mut().enumerate() {
        let same: Vec<f64> = days
            .iter()
            .filter(|d| d.date.weekday().num_days_from_monday() as usize == wd)
            .map(|d| d.qty)
            .collect();
        // Two observations at least, or the factor is noise.
        if same.len() >= 2 {
            *slot = mean(same.into_iter()) / level;
        }
    }
    f
}

fn mean(xs: impl Iterator<Item = f64>) -> f64 {
    let (sum, n) = xs.fold((0.0, 0usize), |(s, n), x| (s + x, n + 1));
    if n == 0 { 0.0 } else { sum / n as f64 }
}

fn std_dev(xs: impl Iterator<Item = f64>) -> f64 {
    let v: Vec<f64> = xs.collect();
    if v.len() < 2 {
        return 0.0;
    }
    let m = mean(v.iter().copied());
    (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (v.len() - 1) as f64).sqrt()
}

/// Weighted absolute percentage error: Σ|actual − forecast| ÷ Σ actual.
pub fn wape(actual: &[f64], forecast: &[f64]) -> Option<f64> {
    let total: f64 = actual.iter().sum();
    if total <= 0.0 {
        return None;
    }
    let err: f64 = actual
        .iter()
        .zip(forecast)
        .map(|(a, f)| (a - f).abs())
        .sum();
    Some(err / total)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Accuracy {
    pub model_wape: f64,
    pub baseline_wape: f64,
}

impl Accuracy {
    pub fn beats_baseline(&self) -> bool {
        self.model_wape < self.baseline_wape
    }
}

/// Fit on everything but the last 7 usable-or-not days, forecast those 7, and
/// compare against a 4-week moving average fitted on the same data.
pub fn holdout_accuracy(history: &[DayDemand]) -> Option<Accuracy> {
    if history.len() < MIN_USABLE + 7 {
        return None;
    }
    let (train, test) = history.split_at(history.len() - 7);
    let test: Vec<&DayDemand> = test.iter().filter(|d| !d.stockout && !d.closed).collect();
    if test.is_empty() {
        return None;
    }
    let start = test[0].date;
    let fc = forecast(train, start, 7, &[]);
    let actual: Vec<f64> = test.iter().map(|d| d.qty).collect();
    let predicted: Vec<f64> = test
        .iter()
        .map(|d| fc.mean_at((d.date - start).num_days() as usize))
        .collect();

    let ma = mean(usable(train).iter().rev().take(WINDOW).map(|d| d.qty));
    let baseline = vec![ma; actual.len()];

    Some(Accuracy {
        model_wape: wape(&actual, &predicted)?,
        baseline_wape: wape(&actual, &baseline)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(date: NaiveDate, qty: f64) -> DayDemand {
        DayDemand {
            date,
            qty,
            stockout: false,
            closed: false,
        }
    }

    fn monday() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 12).unwrap()
    }

    /// Weekdays sell 2, weekends 4, for `weeks` weeks before `monday()`.
    fn weekly(weeks: i64) -> Vec<DayDemand> {
        (0..weeks * 7)
            .map(|i| {
                let d = monday() - Duration::days(weeks * 7 - i);
                let q = if d.weekday().num_days_from_monday() >= 5 {
                    4.0
                } else {
                    2.0
                };
                day(d, q)
            })
            .collect()
    }

    #[test]
    fn learns_the_weekday_shape() {
        let fc = forecast(&weekly(8), monday(), 7, &[]);
        assert!((fc.mean[0] - 2.0).abs() < 1e-9, "monday {}", fc.mean[0]);
        assert!((fc.mean[5] - 4.0).abs() < 1e-9, "saturday {}", fc.mean[5]);
        assert_eq!(fc.confidence, Confidence::High);
    }

    #[test]
    fn upper_bound_is_never_below_the_mean() {
        let fc = forecast(&weekly(8), monday(), 14, &[]);
        assert!(fc.mean.iter().zip(&fc.upper).all(|(m, u)| u >= m));
    }

    #[test]
    fn stockout_days_do_not_drag_the_forecast_down() {
        let mut h = weekly(8);
        let mut censored = h.clone();
        for d in censored.iter_mut().rev().take(5) {
            d.qty = 0.0;
            d.stockout = true;
        }
        let a = forecast(&h, monday(), 7, &[]);
        let b = forecast(&censored, monday(), 7, &[]);
        assert!((a.daily_rate() - b.daily_rate()).abs() < 0.1);
        h.clear();
    }

    #[test]
    fn announced_closures_forecast_zero() {
        let tue = monday() + Duration::days(1);
        let fc = forecast(&weekly(8), monday(), 7, &[tue]);
        assert_eq!(fc.mean[1], 0.0);
        assert_eq!(fc.upper[1], 0.0);
    }

    #[test]
    fn short_history_is_low_confidence() {
        let fc = forecast(&weekly(1), monday(), 7, &[]);
        assert_eq!(fc.confidence, Confidence::Low);
        assert!(fc.method.contains("moving average"));
    }

    #[test]
    fn past_the_horizon_repeats_the_last_week() {
        let fc = forecast(&weekly(8), monday(), 7, &[]);
        assert_eq!(fc.mean_at(12), fc.mean_at(5));
    }

    #[test]
    fn wape_is_relative_absolute_error() {
        assert_eq!(wape(&[10.0, 10.0], &[8.0, 12.0]), Some(0.2));
        assert_eq!(wape(&[0.0], &[1.0]), None);
    }

    #[test]
    fn seasonal_beats_a_flat_average_on_seasonal_data() {
        let acc = holdout_accuracy(&weekly(10)).unwrap();
        assert!(acc.beats_baseline(), "{acc:?}");
    }
}
