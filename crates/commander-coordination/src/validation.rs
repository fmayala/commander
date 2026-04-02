use crate::orchestrator::{ReviewIssue, ValidationResult};
use async_trait::async_trait;

/// A single step in the validation pipeline.
#[async_trait]
pub trait ValidationStep: Send + Sync {
    fn name(&self) -> &str;
    async fn run(&self, task_id: &str, context: &ValidationContext) -> StepResult;
}

pub struct ValidationContext {
    pub working_dir: std::path::PathBuf,
    pub allowed_files: Vec<String>,
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
        // In a real implementation: git diff + filesystem scan against allowed_files.
        // For now, this is a structural placeholder that the management loop can call.
        // The real enforcement for Write/Edit is pre-execution via PathGuard.
        // This catches Bash/MCP side effects.
        let _ = &context.allowed_files;
        StepResult {
            passed: true,
            issues: vec![],
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
