use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    Claude,
    Codex,
}
impl fmt::Display for Agent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        })
    }
}
impl Agent {
    pub fn pinned_version(self) -> &'static str {
        match self {
            Self::Claude => "2.1.278",
            Self::Codex => "0.155.1",
        }
    }
    pub fn version_banner(self) -> String {
        match self {
            Self::Claude => format!("{} (Claude Code)", self.pinned_version()),
            Self::Codex => format!("codex-cli {}", self.pinned_version()),
        }
    }
    /// Only ever passed to an externally isolated guest. Prompt bytes go to stdin.
    pub fn invocation(self, model: &str) -> Vec<String> {
        let mut args: Vec<String> = match self {
            Self::Claude => vec![
                "claude",
                "--print",
                "--verbose",
                "--output-format",
                "stream-json",
                "--dangerously-skip-permissions",
                "--no-session-persistence",
                "--safe-mode",
                "--strict-mcp-config",
                "--mcp-config",
                "{\"mcpServers\":{}}",
                "--no-chrome",
                // sbx exec rejects empty argv elements; the native CLI accepts
                // the equivalent empty option value in --name=value form.
                "--setting-sources=",
                "--disallowedTools",
                "WebSearch,WebFetch",
                "--model",
            ],
            Self::Codex => vec![
                "codex",
                "exec",
                "--json",
                "--color",
                "never",
                "--ephemeral",
                "--ignore-user-config",
                "--ignore-rules",
                "--dangerously-bypass-approvals-and-sandbox",
                "-c",
                "web_search=\"disabled\"",
                "-c",
                "mcp_servers={}",
                "-c",
                "features.apps=false",
                "-c",
                "check_for_update_on_startup=false",
                // sbx's subscription broker uses a sentinel bearer against
                // the ChatGPT Codex endpoint. --ignore-user-config also drops
                // Docker's generated provider, so supply the reviewed routing
                // explicitly. Native ChatGPT login would reject Docker's
                // placeholder auth.json; the backend verifies OAuth mode.
                "-c",
                "model_provider=\"sandboxd\"",
                "-c",
                "model_providers.sandboxd.name=\"Sandbox Proxy\"",
                "-c",
                "model_providers.sandboxd.base_url=\"https://chatgpt.com/backend-api/codex\"",
                "-c",
                "model_providers.sandboxd.experimental_bearer_token=\"oai-oat01-proxy-managed\"",
                "-c",
                "model_providers.sandboxd.requires_openai_auth=false",
                "--model",
            ],
        }
        .into_iter()
        .map(String::from)
        .collect();
        args.push(model.to_string());
        if self == Self::Codex {
            args.push("-".into());
        }
        args
    }
    pub fn settings(self) -> Value {
        let mut settings = json!({"external_sandbox_required": true, "prompt_transport": "stdin_exact_bytes",
            "new_session": true, "provider_web_search": "disabled", "external_mcp": "disabled",
            "host_config": "not_copied", "resume": false, "harness_retries": 0,
            "wall_clock_budget": null, "token_budget": null, "cost_budget": null});
        if self == Self::Codex {
            settings["authentication"] = json!({"provider": "sandboxd", "billing": "chatgpt_subscription", "credential": "broker_sentinel", "requires_observed_oauth_mode": true});
        }
        settings
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Completion {
    NotStarted,
    Normal,
    Interrupted,
    UserAbort,
    InfrastructureFailure,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Outcome {
    pub completion: Completion,
    pub stop_reason: String,
    pub exit_code: Option<i32>,
    pub usage: Option<Value>,
    pub cost_usd: Option<f64>,
    pub transcript_truncated: bool,
}
impl Default for Outcome {
    fn default() -> Self {
        Self {
            completion: Completion::NotStarted,
            stop_reason: "not_delivered".into(),
            exit_code: None,
            usage: None,
            cost_usd: None,
            transcript_truncated: false,
        }
    }
}

#[derive(Debug, Default)]
pub struct Events {
    pub normal: bool,
    pub failed: bool,
    pub malformed: bool,
    pub usage: Option<Value>,
    pub cost_usd: Option<f64>,
    pub model: Option<String>,
    pub interruption: Option<&'static str>,
}
impl Events {
    pub fn consume(&mut self, agent: Agent, bytes: &[u8]) {
        let Ok(v) = serde_json::from_slice::<Value>(bytes) else {
            self.malformed = true;
            return;
        };
        let event = v.get("type").and_then(Value::as_str).unwrap_or("");
        if agent == Agent::Claude && event == "system" && v["subtype"] == "init" {
            self.model = safe_identifier(v["model"].as_str());
        }
        if agent == Agent::Claude && event == "result" {
            self.normal = v["subtype"] == "success" && v["is_error"] == false;
            self.failed = !self.normal;
            self.usage = usage(&v["usage"]);
            self.cost_usd = v["total_cost_usd"]
                .as_f64()
                .filter(|n| n.is_finite() && *n >= 0.0);
        }
        if agent == Agent::Codex && event == "turn.completed" {
            self.normal = true;
            self.failed = false;
            self.usage = usage(&v["usage"]);
        }
        if matches!(event, "error" | "turn.failed")
            || (event == "assistant" && v.get("error").is_some())
        {
            self.failed = true;
            // Do not put arbitrary error text (which can contain credentials) into metadata.
            self.interruption = Some(match v.get("error").and_then(Value::as_str).unwrap_or("") {
                "authentication_failed" => "authentication",
                "rate_limit" => "provider_rate_limit",
                "server_error" => "provider_error",
                _ => "provider_auth_network_or_agent_error_unknown",
            });
        }
    }
    pub fn finish(
        self,
        exit: Option<i32>,
        aborted: bool,
        infrastructure: bool,
        truncated: bool,
    ) -> (Outcome, Option<String>) {
        let (completion, reason) = if aborted {
            (Completion::UserAbort, "user_signal")
        } else if infrastructure {
            (
                Completion::InfrastructureFailure,
                "controller_io_or_process_failure",
            )
        } else if exit == Some(0) && self.normal && !self.failed && !self.malformed {
            (Completion::Normal, "natural_completion")
        } else {
            (
                Completion::Interrupted,
                self.interruption
                    .unwrap_or("missing_or_unsuccessful_terminal_event"),
            )
        };
        (
            Outcome {
                completion,
                stop_reason: reason.into(),
                exit_code: exit,
                usage: self.usage,
                cost_usd: self.cost_usd,
                transcript_truncated: truncated,
            },
            self.model,
        )
    }
}
fn usage(v: &Value) -> Option<Value> {
    let source = v.as_object()?;
    let mut result = serde_json::Map::new();
    for k in [
        "input_tokens",
        "output_tokens",
        "cached_input_tokens",
        "reasoning_output_tokens",
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
    ] {
        if let Some(n) = source.get(k).and_then(Value::as_u64) {
            result.insert(k.into(), n.into());
        }
    }
    if result.is_empty() {
        None
    } else {
        Some(Value::Object(result))
    }
}
fn safe_identifier(s: Option<&str>) -> Option<String> {
    s.filter(|s| {
        s.len() <= 200
            && s.bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-._/:".contains(&c))
    })
    .map(String::from)
}
