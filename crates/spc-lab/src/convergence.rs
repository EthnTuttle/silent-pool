//! Explicit, bounded convergence actions for scenario phases.
//!
//! There is no runtime-initiated mining outside this DSL. Every resume/pause
//! is issued by the runtime through its durable typed command path, and only
//! version 1 with real observed shared hashes can resolve as converged.

use std::time::{Duration, Instant};

use crate::evidence::{
    AtomicTipObservation, TopologyConvergenceEvidence, TOPOLOGY_EVIDENCE_SCHEMA_VERSION,
};
use crate::scenario::ConvergenceSpec;
use serde_json::Value;

pub trait ConvergenceDriver {
    fn observe(&mut self) -> Vec<AtomicTipObservation>;
    fn worker_control(&mut self, resume: bool) -> Result<Vec<Value>, String>;
    fn stop_requested(&self) -> bool;
    fn wait(&mut self, duration: Duration);
}

/// A common hash is a candidate only when every node has a valid atomic view
/// at the same height. A final pause/revalidation is still required.
pub fn shared_tip(views: &[AtomicTipObservation]) -> Option<(u64, String)> {
    let first = views.first()?;
    if !first.atomic || views.is_empty() {
        return None;
    }
    let height = first.height?;
    let hash = first.hash.as_ref()?;
    views
        .iter()
        .all(|view| view.atomic && view.height == Some(height) && view.hash.as_ref() == Some(hash))
        .then(|| (height, hash.clone()))
}

fn evidence(
    spec: &ConvergenceSpec,
    phase: &str,
    status: &str,
    attempts: u32,
    views: Vec<AtomicTipObservation>,
    controls: Vec<Value>,
    unknown_reason: Option<String>,
) -> TopologyConvergenceEvidence {
    TopologyConvergenceEvidence {
        schema_version: TOPOLOGY_EVIDENCE_SCHEMA_VERSION,
        phase: phase.to_string(),
        status: status.to_string(),
        attempts,
        pre_partition_common_anchor: None,
        views,
        controls,
        policy: spec.clone(),
        unknown_reason,
    }
}

/// Drive one explicitly declared convergence action. `resume == false` is a
/// pause command, which does no hashing and is required before the final
/// atomic view. A resume is bounded separately from view observations.
pub fn drive_convergence(
    phase: &str,
    spec: &ConvergenceSpec,
    driver: &mut dyn ConvergenceDriver,
) -> Result<TopologyConvergenceEvidence, String> {
    let started = Instant::now();
    let deadline = started + Duration::from_secs(spec.timeout_secs);
    let mut attempts = 0u32;
    let mut resumes = 0u32;
    let mut quiesced = false;
    let mut controls = Vec::new();
    let mut last_views = Vec::new();

    loop {
        if driver.stop_requested() {
            return Ok(evidence(
                spec,
                phase,
                "unknown",
                attempts,
                last_views,
                controls,
                Some("stop requested before convergence was established".to_string()),
            ));
        }
        if Instant::now() >= deadline {
            return Ok(evidence(
                spec,
                phase,
                "not_converged",
                attempts,
                last_views,
                controls,
                Some("convergence timeout elapsed without a shared atomic tip".to_string()),
            ));
        }
        if attempts >= spec.max_observations {
            return Ok(evidence(
                spec,
                phase,
                "not_converged",
                attempts,
                last_views,
                controls,
                Some("atomic observation budget exhausted without convergence".to_string()),
            ));
        }
        attempts += 1;
        let views = driver.observe();
        last_views = views.clone();
        let shared = shared_tip(&views);
        if shared.is_some() && !quiesced {
            controls.extend(driver.worker_control(false)?);
            quiesced = true;
            // A pause is not the final proof: always re-read after this command.
            driver.wait(Duration::from_secs(1));
            continue;
        }
        if shared.is_some() {
            return Ok(evidence(
                spec,
                phase,
                "converged",
                attempts,
                views,
                controls,
                None,
            ));
        }
        if spec.allow_resume && resumes < spec.max_resume_attempts {
            // Any resume can change worker state. Even a failed/partial resume
            // invalidates quiescence before its command is attempted, so a
            // later shared tip requires a new pause and a fresh atomic read.
            quiesced = false;
            controls.extend(driver.worker_control(true)?);
            resumes += 1;
            // Explicit real SV2 work is required to raise one branch's work.
            worker_wait(driver);
            continue;
        }
        if attempts < spec.max_observations {
            driver.wait(Duration::from_secs(1));
        }
    }

    fn worker_wait(driver: &mut dyn ConvergenceDriver) {
        driver.wait(Duration::from_secs(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    fn view(node: &str, height: u64, hash: &str) -> AtomicTipObservation {
        AtomicTipObservation {
            node: node.to_string(),
            height: Some(height),
            hash: Some(hash.to_string()),
            parent_hash: Some("parent".to_string()),
            atomic: true,
            peers: Some(2),
            error: None,
        }
    }

    fn pair(a: (u64, &str), b: (u64, &str)) -> Vec<AtomicTipObservation> {
        vec![view("node-a", a.0, a.1), view("node-b", b.0, b.1)]
    }

    fn spec(allow_resume: bool, max_observations: u32) -> ConvergenceSpec {
        ConvergenceSpec {
            version: 1,
            timeout_secs: 90,
            max_observations,
            allow_resume,
            max_resume_attempts: u32::from(allow_resume),
        }
    }

    struct FakeDriver {
        observations: VecDeque<Vec<AtomicTipObservation>>,
        fallback: Vec<AtomicTipObservation>,
        calls: Vec<bool>,
        stop: Arc<AtomicBool>,
        stop_on_wait: bool,
        sleep_ms: u64,
    }

    impl FakeDriver {
        fn new(observations: Vec<Vec<AtomicTipObservation>>) -> Self {
            let fallback = observations.last().cloned().unwrap_or_default();
            Self {
                observations: observations.into(),
                fallback,
                calls: Vec::new(),
                stop: Arc::new(AtomicBool::new(false)),
                stop_on_wait: false,
                sleep_ms: 0,
            }
        }
    }

    impl ConvergenceDriver for FakeDriver {
        fn observe(&mut self) -> Vec<AtomicTipObservation> {
            if self.sleep_ms > 0 {
                std::thread::sleep(Duration::from_millis(self.sleep_ms));
            }
            self.observations
                .pop_front()
                .unwrap_or_else(|| self.fallback.clone())
        }

        fn worker_control(&mut self, resume: bool) -> Result<Vec<Value>, String> {
            self.calls.push(resume);
            Ok(vec![serde_json::json!({"resume": resume})])
        }

        fn stop_requested(&self) -> bool {
            self.stop.load(Ordering::SeqCst)
        }

        fn wait(&mut self, _duration: Duration) {
            if self.stop_on_wait && self.observations.is_empty() {
                self.stop.store(true, Ordering::SeqCst);
            }
        }
    }

    #[test]
    fn tied_fork_needs_bounded_real_work_then_paused_revalidation() {
        let mut driver = FakeDriver::new(vec![
            pair((8, "fork-a"), (8, "fork-b")),
            pair((9, "fork-a"), (9, "fork-b")),
            pair((10, "winner"), (10, "winner")),
            pair((10, "winner"), (10, "winner")),
        ]);
        let result = drive_convergence("converge-test", &spec(true, 4), &mut driver).unwrap();
        assert_eq!(result.status, "converged");
        assert_eq!(driver.calls, vec![true, false]);
        assert_eq!(result.attempts, 4);
        assert_eq!(result.policy.max_resume_attempts, 1);
    }

    #[test]
    fn resumed_common_tip_requires_a_new_pause_and_fresh_revalidation() {
        let mut driver = FakeDriver::new(vec![
            pair((8, "tie"), (8, "tie")),
            pair((9, "in-flight"), (8, "tie")),
            pair((10, "winner"), (10, "winner")),
            pair((10, "winner"), (10, "winner")),
        ]);
        let result = drive_convergence("resumed-fork", &spec(true, 4), &mut driver).unwrap();
        assert_eq!(result.status, "converged");
        assert_eq!(
            driver.calls,
            vec![false, true, false],
            "a resume invalidates the old quiesced state; it must pause again"
        );
        assert_eq!(
            result.attempts, 4,
            "there must be an actual fresh observation after the last pause"
        );
        assert!(
            driver.observations.is_empty(),
            "the fourth scripted view, not a reused pre-resume view, certifies convergence"
        );
    }

    #[test]
    fn disabled_resume_does_not_hash_or_get_an_implicit_recovery() {
        let mut driver = FakeDriver::new(vec![pair((8, "fork-a"), (8, "fork-b"))]);
        let mut disabled = spec(false, 2);
        disabled.max_resume_attempts = 0;
        let result = drive_convergence("no-resume", &disabled, &mut driver).unwrap();
        assert_eq!(result.status, "not_converged");
        assert!(
            driver.calls.is_empty(),
            "no resume was permitted: {:?}",
            driver.calls
        );
        assert!(result.views.iter().all(|view| view.atomic));
    }

    #[test]
    fn differing_heights_are_never_a_shared_tip_or_a_fork_certificate() {
        let mut driver = FakeDriver::new(vec![pair((8, "chain-parent"), (9, "descendant"))]);
        let mut disabled = spec(false, 1);
        disabled.max_resume_attempts = 0;
        let result = drive_convergence("height-difference", &disabled, &mut driver).unwrap();
        assert_eq!(result.status, "not_converged");
        assert!(
            driver.calls.is_empty(),
            "different heights alone do not trigger recovery"
        );
        assert!(result.views[0].parent_hash.is_some());
        assert!(result.views[1].parent_hash.is_some());
    }

    #[test]
    fn deadline_is_not_converged_not_an_invented_convergence() {
        let mut driver = FakeDriver::new(vec![pair((8, "fork-a"), (8, "fork-b"))]);
        let mut spec = spec(true, 2);
        spec.timeout_secs = 0;
        let result = drive_convergence("timeout", &spec, &mut driver).unwrap();
        assert_eq!(result.status, "not_converged");
        assert_eq!(result.attempts, 0);
        assert!(result.unknown_reason.unwrap().contains("timeout"));
    }

    #[test]
    fn stop_is_interruptible_and_unknown() {
        let mut driver = FakeDriver::new(vec![
            pair((8, "fork-a"), (8, "fork-b")),
            pair((8, "fork-a"), (8, "fork-b")),
        ]);
        driver.stop_on_wait = true;
        let mut disabled = spec(false, 90);
        disabled.max_resume_attempts = 0;
        let result = drive_convergence("stop-interruptible", &disabled, &mut driver).unwrap();
        assert_eq!(result.status, "unknown");
        assert!(result.attempts < disabled.max_observations);
        assert!(result.unknown_reason.unwrap().contains("stop"));
    }
}
