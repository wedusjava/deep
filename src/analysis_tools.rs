use anyhow::{Result, bail};
use serde_json::{Value, json};

const MAX_DIFF_LINES: usize = 500;

pub fn statistics(
    operation: &str,
    values: &[f64],
    other_values: Option<&[f64]>,
    percentile: Option<f64>,
) -> Result<Value> {
    if values.is_empty() {
        bail!("statistics requires at least one value");
    }
    if values.iter().any(|value| !value.is_finite()) {
        bail!("statistics values must be finite numbers");
    }

    let result = match operation {
        "sum" => sum(values),
        "mean" => mean(values),
        "median" => median(values),
        "min" => values.iter().copied().fold(f64::INFINITY, f64::min),
        "max" => values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        "percentage_change" => {
            if values.len() != 2 {
                bail!("percentage_change requires exactly two values: old and new");
            }
            if values[0] == 0.0 {
                bail!("percentage_change is undefined when the old value is zero");
            }
            ((values[1] - values[0]) / values[0]) * 100.0
        }
        "standard_deviation" => standard_deviation(values),
        "percentile" => percentile_value(
            values,
            percentile.ok_or_else(|| anyhow::anyhow!("percentile requires p"))?,
        )?,
        "correlation" => correlation(
            values,
            other_values.ok_or_else(|| anyhow::anyhow!("correlation requires other_values"))?,
        )?,
        _ => bail!("unknown statistics operation: {operation}"),
    };

    Ok(json!({"operation": operation, "result": result}))
}

pub fn text_diff(left: &str, right: &str) -> Result<String> {
    let left_lines = left.lines().collect::<Vec<_>>();
    let right_lines = right.lines().collect::<Vec<_>>();
    if left_lines.len() > MAX_DIFF_LINES || right_lines.len() > MAX_DIFF_LINES {
        bail!("text_diff is limited to {MAX_DIFF_LINES} lines per input");
    }

    let rows = left_lines.len() + 1;
    let cols = right_lines.len() + 1;
    let mut lcs = vec![0usize; rows * cols];
    let index = |i: usize, j: usize| i * cols + j;

    for i in (0..left_lines.len()).rev() {
        for j in (0..right_lines.len()).rev() {
            lcs[index(i, j)] = if left_lines[i] == right_lines[j] {
                1 + lcs[index(i + 1, j + 1)]
            } else {
                lcs[index(i + 1, j)].max(lcs[index(i, j + 1)])
            };
        }
    }

    let mut output = String::new();
    let mut i = 0usize;
    let mut j = 0usize;
    while i < left_lines.len() || j < right_lines.len() {
        if i < left_lines.len() && j < right_lines.len() && left_lines[i] == right_lines[j] {
            output.push_str("  ");
            output.push_str(left_lines[i]);
            output.push('\n');
            i += 1;
            j += 1;
        } else if j < right_lines.len()
            && (i == left_lines.len() || lcs[index(i, j + 1)] >= lcs[index(i + 1, j)])
        {
            output.push_str("+ ");
            output.push_str(right_lines[j]);
            output.push('\n');
            j += 1;
        } else {
            output.push_str("- ");
            output.push_str(left_lines[i]);
            output.push('\n');
            i += 1;
        }
    }

    Ok(output)
}

fn sum(values: &[f64]) -> f64 {
    values.iter().sum()
}

fn mean(values: &[f64]) -> f64 {
    sum(values) / values.len() as f64
}

fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[middle - 1] + sorted[middle]) / 2.0
    } else {
        sorted[middle]
    }
}

fn standard_deviation(values: &[f64]) -> f64 {
    let average = mean(values);
    let variance = values
        .iter()
        .map(|value| (value - average).powi(2))
        .sum::<f64>()
        / values.len() as f64;
    variance.sqrt()
}

fn percentile_value(values: &[f64], p: f64) -> Result<f64> {
    if !(0.0..=100.0).contains(&p) {
        bail!("percentile p must be between 0 and 100");
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    if sorted.len() == 1 {
        return Ok(sorted[0]);
    }

    let rank = (p / 100.0) * (sorted.len() - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    if lower == upper {
        Ok(sorted[lower])
    } else {
        let weight = rank - lower as f64;
        Ok(sorted[lower] * (1.0 - weight) + sorted[upper] * weight)
    }
}

fn correlation(values: &[f64], other: &[f64]) -> Result<f64> {
    if values.len() != other.len() || values.len() < 2 {
        bail!("correlation requires two arrays of equal length with at least two values");
    }
    if other.iter().any(|value| !value.is_finite()) {
        bail!("correlation values must be finite numbers");
    }

    let mean_x = mean(values);
    let mean_y = mean(other);
    let mut numerator = 0.0;
    let mut sum_x = 0.0;
    let mut sum_y = 0.0;
    for (&x, &y) in values.iter().zip(other.iter()) {
        let dx = x - mean_x;
        let dy = y - mean_y;
        numerator += dx * dy;
        sum_x += dx * dx;
        sum_y += dy * dy;
    }
    let denominator = (sum_x * sum_y).sqrt();
    if denominator == 0.0 {
        bail!("correlation is undefined for a constant series");
    }
    Ok(numerator / denominator)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statistics_are_deterministic() {
        let values = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(statistics("sum", &values, None, None).unwrap()["result"], 10.0);
        assert_eq!(statistics("median", &values, None, None).unwrap()["result"], 2.5);
        assert_eq!(
            statistics("percentile", &values, None, Some(50.0)).unwrap()["result"],
            2.5
        );
    }

    #[test]
    fn diff_marks_insertions_and_deletions() {
        let diff = text_diff("a\nb", "a\nc").unwrap();
        assert!(diff.contains("- b"));
        assert!(diff.contains("+ c"));
    }
}
