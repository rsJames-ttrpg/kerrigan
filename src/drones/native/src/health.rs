use std::time::Duration;

use crate::pipeline::Stage;

#[derive(Debug, Clone)]
pub struct HealthCheck {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub expected_exit_code: i32,
    pub timeout: Duration,
    pub required: bool,
}

#[derive(Debug)]
pub struct HealthCheckResult {
    pub name: String,
    pub passed: bool,
    pub required: bool,
    pub output: String,
    pub duration_ms: u64,
}

#[derive(Debug)]
pub struct HealthReport {
    pub checks: Vec<HealthCheckResult>,
}

impl HealthReport {
    pub fn all_required_passed(&self) -> bool {
        self.checks.iter().all(|c| !c.required || c.passed)
    }

    pub fn summary(&self) -> String {
        let failed: Vec<_> = self
            .checks
            .iter()
            .filter(|c| !c.passed)
            .map(|c| {
                format!(
                    "{} ({})",
                    c.name,
                    if c.required { "required" } else { "optional" }
                )
            })
            .collect();
        if failed.is_empty() {
            "all checks passed".into()
        } else {
            format!("failed: {}", failed.join(", "))
        }
    }
}

pub async fn run_health_checks(checks: &[HealthCheck]) -> HealthReport {
    let mut results = Vec::new();
    for check in checks {
        let start = std::time::Instant::now();
        let output = tokio::time::timeout(
            check.timeout,
            tokio::process::Command::new(&check.command)
                .args(&check.args)
                .output(),
        )
        .await;

        let (passed, output_str) = match output {
            Ok(Ok(o)) => (
                o.status.code() == Some(check.expected_exit_code),
                String::from_utf8_lossy(&o.stdout).to_string()
                    + &String::from_utf8_lossy(&o.stderr),
            ),
            Ok(Err(e)) => (false, format!("failed to execute: {e}")),
            Err(_) => (false, "timed out".to_string()),
        };

        results.push(HealthCheckResult {
            name: check.name.clone(),
            passed,
            required: check.required,
            output: output_str,
            duration_ms: start.elapsed().as_millis() as u64,
        });
    }
    HealthReport { checks: results }
}

pub fn checks_for_stage(stage: &Stage) -> Vec<HealthCheck> {
    let mut checks = vec![
        HealthCheck {
            name: "cargo".into(),
            command: "cargo".into(),
            args: vec!["--version".into()],
            expected_exit_code: 0,
            timeout: Duration::from_secs(10),
            required: true,
        },
        HealthCheck {
            name: "rustc".into(),
            command: "rustc".into(),
            args: vec!["--version".into()],
            expected_exit_code: 0,
            timeout: Duration::from_secs(10),
            required: true,
        },
        HealthCheck {
            name: "git".into(),
            command: "git".into(),
            args: vec!["--version".into()],
            expected_exit_code: 0,
            timeout: Duration::from_secs(10),
            required: true,
        },
    ];

    match stage {
        Stage::Implement => {
            checks.push(HealthCheck {
                name: "build".into(),
                command: "cargo".into(),
                args: vec!["check".into()],
                expected_exit_code: 0,
                timeout: Duration::from_secs(300),
                required: true,
            });
            checks.push(HealthCheck {
                name: "tests".into(),
                command: "cargo".into(),
                args: vec!["test".into()],
                expected_exit_code: 0,
                timeout: Duration::from_secs(600),
                required: true,
            });
            checks.push(HealthCheck {
                name: "gh".into(),
                command: "gh".into(),
                args: vec!["--version".into()],
                expected_exit_code: 0,
                timeout: Duration::from_secs(10),
                required: true,
            });
        }
        Stage::Review => {
            checks.push(HealthCheck {
                name: "build".into(),
                command: "cargo".into(),
                args: vec!["check".into()],
                expected_exit_code: 0,
                timeout: Duration::from_secs(300),
                required: true,
            });
            checks.push(HealthCheck {
                name: "gh".into(),
                command: "gh".into(),
                args: vec!["--version".into()],
                expected_exit_code: 0,
                timeout: Duration::from_secs(10),
                required: true,
            });
        }
        _ => {}
    }

    checks.push(HealthCheck {
        name: "creep".into(),
        command: "creep-cli".into(),
        args: vec!["--version".into()],
        expected_exit_code: 0,
        timeout: Duration::from_secs(10),
        required: false,
    });

    checks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_stages_have_base_checks() {
        for stage in [
            Stage::Spec,
            Stage::Plan,
            Stage::Implement,
            Stage::Review,
            Stage::Evolve,
            Stage::Freeform,
        ] {
            let checks = checks_for_stage(&stage);
            let names: Vec<&str> = checks.iter().map(|c| c.name.as_str()).collect();
            assert!(names.contains(&"cargo"), "{stage:?} missing cargo");
            assert!(names.contains(&"rustc"), "{stage:?} missing rustc");
            assert!(names.contains(&"git"), "{stage:?} missing git");
            assert!(names.contains(&"creep"), "{stage:?} missing creep");
        }
    }

    #[test]
    fn implement_stage_has_build_and_test_checks() {
        let checks = checks_for_stage(&Stage::Implement);
        let names: Vec<&str> = checks.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"build"));
        assert!(names.contains(&"tests"));
        assert!(names.contains(&"gh"));
    }

    #[test]
    fn review_stage_has_build_and_gh_checks() {
        let checks = checks_for_stage(&Stage::Review);
        let names: Vec<&str> = checks.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"build"));
        assert!(names.contains(&"gh"));
        assert!(!names.contains(&"tests"));
    }

    #[test]
    fn spec_stage_has_only_base_checks() {
        let checks = checks_for_stage(&Stage::Spec);
        assert_eq!(checks.len(), 4); // cargo, rustc, git, creep
    }

    #[test]
    fn creep_is_optional() {
        let checks = checks_for_stage(&Stage::Spec);
        let creep = checks.iter().find(|c| c.name == "creep").unwrap();
        assert!(!creep.required);
    }

    #[test]
    fn base_checks_are_required() {
        let checks = checks_for_stage(&Stage::Spec);
        for check in &checks {
            if check.name != "creep" {
                assert!(check.required, "{} should be required", check.name);
            }
        }
    }

    #[test]
    fn report_all_passed() {
        let report = HealthReport {
            checks: vec![
                HealthCheckResult {
                    name: "a".into(),
                    passed: true,
                    required: true,
                    output: String::new(),
                    duration_ms: 1,
                },
                HealthCheckResult {
                    name: "b".into(),
                    passed: true,
                    required: false,
                    output: String::new(),
                    duration_ms: 1,
                },
            ],
        };
        assert!(report.all_required_passed());
        assert_eq!(report.summary(), "all checks passed");
    }

    #[test]
    fn report_optional_failed() {
        let report = HealthReport {
            checks: vec![
                HealthCheckResult {
                    name: "a".into(),
                    passed: true,
                    required: true,
                    output: String::new(),
                    duration_ms: 1,
                },
                HealthCheckResult {
                    name: "b".into(),
                    passed: false,
                    required: false,
                    output: String::new(),
                    duration_ms: 1,
                },
            ],
        };
        assert!(report.all_required_passed());
        assert!(report.summary().contains("b (optional)"));
    }

    #[test]
    fn report_required_failed() {
        let report = HealthReport {
            checks: vec![HealthCheckResult {
                name: "critical".into(),
                passed: false,
                required: true,
                output: String::new(),
                duration_ms: 1,
            }],
        };
        assert!(!report.all_required_passed());
        assert!(report.summary().contains("critical (required)"));
    }

    #[tokio::test]
    async fn run_health_checks_true_command() {
        let checks = vec![HealthCheck {
            name: "true".into(),
            command: "true".into(),
            args: vec![],
            expected_exit_code: 0,
            timeout: Duration::from_secs(5),
            required: true,
        }];
        let report = run_health_checks(&checks).await;
        assert!(report.all_required_passed());
        assert_eq!(report.checks.len(), 1);
        assert!(report.checks[0].passed);
    }

    #[tokio::test]
    async fn run_health_checks_false_command() {
        let checks = vec![HealthCheck {
            name: "false".into(),
            command: "false".into(),
            args: vec![],
            expected_exit_code: 0,
            timeout: Duration::from_secs(5),
            required: true,
        }];
        let report = run_health_checks(&checks).await;
        assert!(!report.all_required_passed());
        assert!(!report.checks[0].passed);
    }

    #[tokio::test]
    async fn run_health_checks_nonexistent_command() {
        let checks = vec![HealthCheck {
            name: "missing".into(),
            command: "nonexistent-command-12345".into(),
            args: vec![],
            expected_exit_code: 0,
            timeout: Duration::from_secs(5),
            required: true,
        }];
        let report = run_health_checks(&checks).await;
        assert!(!report.all_required_passed());
        assert!(report.checks[0].output.contains("failed to execute"));
    }
}
