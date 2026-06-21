//! 통계 — McNemar 검정(Q1)과 부트스트랩 신뢰구간(Q2) (M4-T04·T05).
//!
//! 외부 크레이트 없이 결정론적으로 구현한다(재현성). 부트스트랩 난수는 고정 시드의
//! xorshift로 만들어, 같은 입력·시드면 항상 같은 CI가 나온다.
//!
//! **측정의 정직성(설계 5.3절):** 점추정만 내지 않는다. 정확성은 검정의 p-값으로,
//! 절감은 신뢰구간으로 보고해 불확실성을 함께 드러낸다.

/// McNemar 검정 결과(짝지은 이진 결과의 불일치 분석).
#[derive(Debug, Clone, PartialEq)]
pub struct McNemar {
    /// PTC만 통과(baseline 실패)한 task 수.
    pub ptc_only: usize,
    /// baseline만 통과(PTC 실패)한 task 수.
    pub baseline_only: usize,
    /// 카이제곱 통계량(정확검정을 썼으면 `None`).
    pub statistic: Option<f64>,
    pub p_value: f64,
    /// 불일치 쌍이 적어(<25) 정확 이항검정을 썼는가.
    pub exact: bool,
}

impl McNemar {
    /// 유의수준 alpha에서 PTC가 유의하게 **악화**됐는가
    /// (유의한 차이 AND baseline만 통과가 더 많음). Q1 게이트의 반대 조건.
    pub fn degraded(&self, alpha: f64) -> bool {
        self.p_value < alpha && self.baseline_only > self.ptc_only
    }
}

/// 짝지은 (ptc_pass, baseline_pass)들에서 McNemar 검정을 수행한다.
pub fn mcnemar(pairs: &[(bool, bool)]) -> McNemar {
    let ptc_only = pairs.iter().filter(|(p, b)| *p && !*b).count();
    let baseline_only = pairs.iter().filter(|(p, b)| !*p && *b).count();
    let discordant = ptc_only + baseline_only;

    if discordant < 25 {
        McNemar {
            ptc_only,
            baseline_only,
            statistic: None,
            p_value: exact_binomial_two_sided(discordant, ptc_only.min(baseline_only)),
            exact: true,
        }
    } else {
        // 연속성 보정 카이제곱(자유도 1).
        let diff = (ptc_only as f64 - baseline_only as f64).abs();
        let statistic = (diff - 1.0).max(0.0).powi(2) / discordant as f64;
        McNemar {
            ptc_only,
            baseline_only,
            statistic: Some(statistic),
            p_value: chi_square_sf_df1(statistic),
            exact: false,
        }
    }
}

/// 양측 정확 이항검정 p-값(p=0.5): `2·Σ_{i=0}^{k} C(n,i)·0.5^n`, 1로 상한.
fn exact_binomial_two_sided(n: usize, k: usize) -> f64 {
    if n == 0 {
        return 1.0;
    }
    let tail: f64 = (0..=k).map(|i| binom(n, i) as f64).sum();
    (2.0 * tail * 0.5f64.powi(n as i32)).min(1.0)
}

/// 이항계수 C(n, k). n<25 범위에서 u64로 안전하다.
fn binom(n: usize, k: usize) -> u64 {
    let k = k.min(n - k);
    let mut result: u64 = 1;
    for i in 0..k {
        result = result * (n - i) as u64 / (i as u64 + 1);
    }
    result
}

/// 카이제곱(자유도 1) 생존함수: `erfc(sqrt(x/2))`.
fn chi_square_sf_df1(x: f64) -> f64 {
    if x <= 0.0 {
        return 1.0;
    }
    erfc((x / 2.0).sqrt())
}

/// erfc 근사(Abramowitz–Stegun 7.1.26). x>=0에서 충분히 정확하다.
fn erfc(x: f64) -> f64 {
    let t = 1.0 / (1.0 + 0.3275911 * x);
    let poly = t
        * (0.254829592
            + t * (-0.284496736 + t * (1.421413741 + t * (-1.453152027 + t * 1.061405429))));
    poly * (-x * x).exp()
}

/// 신뢰구간(점추정 + 하한·상한).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ci {
    pub point: f64,
    pub lower: f64,
    pub upper: f64,
}

/// 짝지은 (numer, denom)에서 `Σnumer/Σdenom` 비율의 부트스트랩 95% CI.
///
/// task를 복원추출로 재표집해 비율 분포를 만들고 2.5/97.5 백분위를 취한다.
/// PTC/baseline 비율이면 상한 < 1.0 이 "유의한 절감"을 뜻한다(Q2).
pub fn bootstrap_ratio_ci(numer: &[f64], denom: &[f64], samples: usize, seed: u64) -> Ci {
    assert_eq!(numer.len(), denom.len(), "짝지은 표본이어야 함");
    let point = ratio(numer, denom);
    let n = numer.len();
    if n == 0 || samples == 0 {
        return Ci {
            point,
            lower: point,
            upper: point,
        };
    }

    let mut rng = Rng::new(seed);
    let mut ratios = Vec::with_capacity(samples);
    for _ in 0..samples {
        let mut sum_n = 0.0;
        let mut sum_d = 0.0;
        for _ in 0..n {
            let idx = rng.below(n);
            sum_n += numer[idx];
            sum_d += denom[idx];
        }
        ratios.push(if sum_d == 0.0 { 0.0 } else { sum_n / sum_d });
    }
    ratios.sort_by(|a, b| a.partial_cmp(b).unwrap());
    Ci {
        point,
        lower: percentile(&ratios, 0.025),
        upper: percentile(&ratios, 0.975),
    }
}

fn ratio(numer: &[f64], denom: &[f64]) -> f64 {
    let d: f64 = denom.iter().sum();
    if d == 0.0 {
        0.0
    } else {
        numer.iter().sum::<f64>() / d
    }
}

/// 정렬된 표본의 백분위(최근접 순위). q는 0..1.
fn percentile(sorted: &[f64], q: f64) -> f64 {
    let n = sorted.len();
    let rank = (q * n as f64).floor() as usize;
    sorted[rank.min(n - 1)]
}

/// 결정론적 xorshift64 PRNG(부트스트랩 재현성용).
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        // 0 시드는 xorshift에서 영구 0이므로 비0으로 보정.
        Rng(if seed == 0 { 0x9E3779B97F4A7C15 } else { seed })
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next_u64() % bound as u64) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_discordance_is_not_significant() {
        let pairs = vec![(true, true); 10];
        let result = mcnemar(&pairs);
        assert_eq!((result.ptc_only, result.baseline_only), (0, 0));
        assert!(result.exact);
        assert_eq!(result.p_value, 1.0);
        assert!(!result.degraded(0.05));
    }

    #[test]
    fn ptc_advantage_is_not_a_degradation() {
        // PTC만 통과 5건, baseline만 통과 0건 → 악화 아님.
        let mut pairs = vec![(true, false); 5];
        pairs.extend(vec![(true, true); 10]);
        let result = mcnemar(&pairs);
        assert_eq!((result.ptc_only, result.baseline_only), (5, 0));
        assert!(!result.degraded(0.05));
    }

    #[test]
    fn exact_binomial_matches_known_value() {
        // n=6, k=1 → 2*(C(6,0)+C(6,1))*0.5^6 = 14/64 = 0.21875.
        let p = exact_binomial_two_sided(6, 1);
        assert!((p - 0.21875).abs() < 1e-9, "got {p}");
    }

    #[test]
    fn large_discordance_uses_chi_square() {
        // baseline만 통과 30건 → 정확검정 임계 초과, 강한 유의.
        let pairs = vec![(false, true); 30];
        let result = mcnemar(&pairs);
        assert!(!result.exact);
        assert!(result.statistic.is_some());
        assert!(result.p_value < 0.001, "p={}", result.p_value);
        assert!(result.degraded(0.05));
    }

    #[test]
    fn chi_square_sf_is_sane() {
        // 자유도 1에서 3.841 근방의 생존함수 ≈ 0.05.
        let p = chi_square_sf_df1(3.841);
        assert!((p - 0.05).abs() < 0.005, "p={p}");
    }

    #[test]
    fn bootstrap_ci_brackets_a_clear_ratio() {
        // PTC 호출 1, baseline 6 → 비율 1/6, 상한 < 1.
        let ptc = vec![1.0; 8];
        let baseline = vec![6.0; 8];
        let ci = bootstrap_ratio_ci(&ptc, &baseline, 2000, 42);
        assert!((ci.point - 1.0 / 6.0).abs() < 1e-9);
        assert!(ci.lower <= ci.point && ci.point <= ci.upper);
        assert!(ci.upper < 1.0, "상한 {} < 1 이어야", ci.upper);
    }

    #[test]
    fn bootstrap_is_deterministic_for_a_seed() {
        let ptc = vec![1.0, 2.0, 1.5, 1.0, 2.0];
        let baseline = vec![6.0, 7.0, 5.0, 6.0, 8.0];
        let a = bootstrap_ratio_ci(&ptc, &baseline, 1000, 7);
        let b = bootstrap_ratio_ci(&ptc, &baseline, 1000, 7);
        assert_eq!(a, b);
    }

    #[test]
    fn varied_ratios_widen_the_interval() {
        // 변동이 있으면 하한<상한.
        let ptc = vec![1.0, 3.0, 1.0, 5.0, 1.0, 4.0];
        let baseline = vec![6.0, 6.0, 6.0, 6.0, 6.0, 6.0];
        let ci = bootstrap_ratio_ci(&ptc, &baseline, 3000, 99);
        assert!(ci.lower < ci.upper);
    }
}
