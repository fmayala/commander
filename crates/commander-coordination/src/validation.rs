use crate::orchestrator::{ReviewIssue, ValidationResult};
use async_trait::async_trait;
use glob::Pattern;

/// A single step in the validation pipeline.
#[async_trait]
pub trait ValidationStep: Send + Sync {
    fn name(&self) -> &str;
    async fn run(&self, task_id: &str, context: &ValidationContext) -> StepResult;
}

pub struct ValidationContext {
    pub working_dir: std::path::PathBuf,
    pub allowed_files: Vec<String>,
    pub require_in_scope_changes: bool,
    pub baseline_paths: Vec<String>,
    pub ignored_prefixes: Vec<String>,
    pub test_command: Option<String>,
}

pub struct StepResult {
    pub passed: bool,
    pub issues: Vec<ReviewIssue>,
}

/// Configurable validation pipeline.
pub struct ValidationPipeline {
    steps: Vec<Box<dyn ValidationStep>>,
    pub max_fix_cycles: u32,
}

impl ValidationPipeline {
    pub fn new(max_fix_cycles: u32) -> Self {
        Self {
            steps: Vec::new(),
            max_fix_cycles,
        }
    }

    pub fn add_step(&mut self, step: Box<dyn ValidationStep>) {
        self.steps.push(step);
    }

    pub async fn run(&self, task_id: &str, context: &ValidationContext) -> ValidationResult {
        let mut all_issues = Vec::new();
        let mut all_passed = true;

        for step in &self.steps {
            let result = step.run(task_id, context).await;
            if !result.passed {
                all_passed = false;
            }
            all_issues.extend(result.issues);
        }

        ValidationResult {
            passed: all_passed,
            issues: all_issues,
        }
    }
}

/// BoundaryCheck: post-execution audit. Verifies all file changes are within allowed scope.
pub struct BoundaryCheckStep;

#[async_trait]
impl ValidationStep for BoundaryCheckStep {
    fn name(&self) -> &str {
        "boundary_check"
    }

    async fn run(&self, _task_id: &str, context: &ValidationContext) -> StepResult {
        if context.allowed_files.is_empty() {
            return StepResult {
                passed: true,
                issues: vec![],
            };
        }

        let patterns: Vec<Pattern> = context
            .allowed_files
            .iter()
            .filter_map(|p| Pattern::new(p).ok())
            .collect();

        if patterns.is_empty() {
            return StepResult {
                passed: false,
                issues: vec![ReviewIssue {
                    file: String::new(),
                    severity: crate::orchestrator::Severity::Error,
                    category: crate::orchestrator::Category::Boundary,
                    description: "boundary check could not compile allowed file patterns".into(),
                    fix_attempts: 0,
                }],
            };
        }

        let output = tokio::process::Command::new("git")
            .arg("status")
            .arg("--porcelain")
            .arg("--untracked-files=all")
            .current_dir(&context.working_dir)
            .output()
            .await;

        let out = match output {
            Ok(out) => out,
            Err(e) => {
                return StepResult {
                    passed: false,
                    issues: vec![ReviewIssue {
                        file: String::new(),
                        severity: crate::orchestrator::Severity::Error,
                        category: crate::orchestrator::Category::Boundary,
                        description: format!("boundary check failed to run git status: {e}"),
                        fix_attempts: 0,
                    }],
                };
            }
        };

        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return StepResult {
                passed: false,
                issues: vec![ReviewIssue {
                    file: String::new(),
                    severity: crate::orchestrator::Severity::Error,
                    category: crate::orchestrator::Category::Boundary,
                    description: format!(
                        "boundary check git status failed (exit {}): {}",
                        out.status.code().unwrap_or(-1),
                        stderr.chars().take(300).collect::<String>()
                    ),
                    fix_attempts: 0,
                }],
            };
        }

        let stdout = String::from_utf8_lossy(&out.stdout);
        let baseline: std::collections::HashSet<String> =
            context.baseline_paths.iter().cloned().collect();
        let changed_paths = parse_git_status_paths(&stdout)
            .into_iter()
            .filter(|path| !baseline.contains(path))
            .filter(|path| !is_ignored_path(path, &context.ignored_prefixes))
            .collect::<Vec<_>>();

        if changed_paths.is_empty() && context.require_in_scope_changes {
            return StepResult {
                passed: false,
                issues: vec![ReviewIssue {
                    file: String::new(),
                    severity: crate::orchestrator::Severity::Error,
                    category: crate::orchestrator::Category::Boundary,
                    description: "no in-scope file changes detected".into(),
                    fix_attempts: 0,
                }],
            };
        }

        let mut issues = Vec::new();
        for path in changed_paths {
            let allowed = patterns.iter().any(|p| p.matches(&path));
            if !allowed {
                issues.push(ReviewIssue {
                    file: path.clone(),
                    severity: crate::orchestrator::Severity::Error,
                    category: crate::orchestrator::Category::Boundary,
                    description: format!("modified path outside allowed scope: {path}"),
                    fix_attempts: 0,
                });
            }
        }

        StepResult {
            passed: issues.is_empty(),
            issues,
        }
    }
}

/// RunTests: execute a test command and check exit code.
pub struct RunTestsStep;

#[async_trait]
impl ValidationStep for RunTestsStep {
    fn name(&self) -> &str {
        "run_tests"
    }

    async fn run(&self, _task_id: &str, context: &ValidationContext) -> StepResult {
        let Some(ref cmd) = context.test_command else {
            return StepResult {
                passed: true,
                issues: vec![],
            };
        };

        let output = tokio::process::Command::new("bash")
            .arg("-c")
            .arg(cmd)
            .current_dir(&context.working_dir)
            .output()
            .await;

        match output {
            Ok(out) if out.status.success() => StepResult {
                passed: true,
                issues: vec![],
            },
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                StepResult {
                    passed: false,
                    issues: vec![ReviewIssue {
                        file: String::new(),
                        severity: crate::orchestrator::Severity::Error,
                        category: crate::orchestrator::Category::Integration,
                        description: format!(
                            "tests failed (exit {}): {}",
                            out.status.code().unwrap_or(-1),
                            stderr.chars().take(500).collect::<String>()
                        ),
                        fix_attempts: 0,
                    }],
                }
            }
            Err(e) => StepResult {
                passed: false,
                issues: vec![ReviewIssue {
                    file: String::new(),
                    severity: crate::orchestrator::Severity::Error,
                    category: crate::orchestrator::Category::Integration,
                    description: format!("failed to run tests: {e}"),
                    fix_attempts: 0,
                }],
            },
        }
    }
}

pub fn parse_git_status_paths(status_output: &str) -> Vec<String> {
    status_output
        .lines()
        .filter_map(|line| {
            let raw = line.get(3..)?.trim();
            if raw.is_empty() {
                return None;
            }

            let path = raw
                .split_once(" -> ")
                .map(|(_, new_path)| new_path)
                .unwrap_or(raw)
                .trim_matches('"')
                .to_string();
            if path.is_empty() {
                None
            } else {
                Some(path)
            }
        })
        .collect()
}

fn is_ignored_path(path: &str, prefixes: &[String]) -> bool {
    prefixes.iter().any(|prefix| path.starts_with(prefix))
}
