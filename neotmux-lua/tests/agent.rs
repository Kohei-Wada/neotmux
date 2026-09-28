//! herdr's agent identification, as `neotmux.agent` runs it. The tests are herdr's own
//! (src/detect/mod.rs, herdrdev/herdr, Apache-2.0), their inputs and expectations carried over:
//! an agent is its label here, and a job's answer is the agent alone rather than the agent and
//! the name it was found under. herdr's test of a symlink resolved to its target is not here —
//! Lua cannot read the filesystem, so `neotmux.agent` does not resolve one.

mod support;

use support::{built, keys, logged};

/// herdr's shell integration for PowerShell, which a plain shell pane carries in its argv and
/// must still be read as a shell.
const WINDOWS_POWERSHELL_SHELL_INTEGRATION_COMMAND: &str = r"if ($null -eq $global:__HerdrOriginalPrompt) { $global:__HerdrOriginalPrompt = $function:prompt; function global:prompt { $out = @(& $global:__HerdrOriginalPrompt) -join ' '; $loc = $ExecutionContext.SessionState.Path.CurrentLocation; if ($loc.Provider.Name -eq 'FileSystem') { try { [Environment]::CurrentDirectory = $loc.ProviderPath } catch {}; $esc = [string][char]27; $out += $esc + ']9;9;' + $loc.ProviderPath + $esc + '\' }; $out } }";

struct Process {
    pid: i32,
    name: String,
    argv: Option<Vec<String>>,
}

struct Job {
    process_group_id: i32,
    processes: Vec<Process>,
}

fn foreground_process(pid: i32, name: &str, argv: &[&str]) -> Process {
    Process {
        pid,
        name: name.to_string(),
        argv: Some(argv.iter().map(|arg| (*arg).to_string()).collect()),
    }
}

/// herdr's cmdline fallback, as a job: a process called something else whose argv is `token`.
fn argv0_only(token: &str) -> Job {
    Job {
        process_group_id: 1,
        processes: vec![foreground_process(1, "MainThread", &[token])],
    }
}

fn lua_string(text: &str) -> String {
    let mut level = 0;
    while text.contains(&format!("]{}]", "=".repeat(level))) || text.ends_with(']') && level == 0 {
        level += 1;
    }
    let eq = "=".repeat(level);
    format!("[{eq}[{text}]{eq}]")
}

/// Runs `expr` in Lua with `neotmux.agent` as `agent`, and answers what it came to.
fn ask(expr: &str) -> Option<String> {
    let (_dir, sub) = built(&format!(
        "local agent = require('neotmux.agent')\n\
         neotmux.on_key(function(key)\n\
           if key == 'J' then\n\
             neotmux.log.info('=' .. tostring({expr}))\n\
             return true\n\
           end\n\
           return false\n\
         end)\n"
    ));
    keys(&sub, &["J"]);
    let said = logged(&sub);
    let answer = said
        .last()
        .expect("an answer")
        .strip_prefix('=')
        .expect("ours");
    (answer != "nil").then(|| answer.to_string())
}

fn identify_in_job(job: &Job) -> Option<String> {
    let processes: Vec<String> = job
        .processes
        .iter()
        .map(|p| {
            let argv = p.argv.as_ref().map_or(String::new(), |argv| {
                let parts: Vec<String> = argv.iter().map(|a| lua_string(a)).collect();
                format!(", argv = {{ {} }}", parts.join(", "))
            });
            format!(
                "{{ pid = {}, name = {}{argv} }}",
                p.pid,
                lua_string(&p.name)
            )
        })
        .collect();
    ask(&format!(
        "agent.identify_job({{ group = {}, processes = {{ {} }} }})",
        job.process_group_id,
        processes.join(", ")
    ))
}

fn identify_name(name: &str) -> Option<&'static str> {
    ask(&format!("agent.identify_name({})", lua_string(name)))
        .map(|s| &*Box::leak(s.into_boxed_str()))
}

#[test]
fn identify_known_agents() {
    assert_eq!(identify_name("pi"), Some("pi"));
    assert_eq!(identify_name("claude"), Some("claude"));
    assert_eq!(identify_name("claude-code"), Some("claude"));
    assert_eq!(identify_name("codex"), Some("codex"));
    assert_eq!(identify_name("gemini"), Some("gemini"));
    assert_eq!(identify_name("cursor"), Some("cursor"));
    assert_eq!(identify_name("cursor-agent"), Some("cursor"));
    assert_eq!(identify_name("devin"), Some("devin"));
    assert_eq!(identify_name("devin-cli"), Some("devin"));
    assert_eq!(identify_name("agy"), Some("agy"));
    assert_eq!(identify_name("antigravity-cli"), Some("agy"));
    assert_eq!(identify_name("cline"), Some("cline"));
    assert_eq!(identify_name("omp"), Some("omp"));
    assert_eq!(identify_name("mastracode"), Some("mastracode"));
    assert_eq!(identify_name("mastra-code"), Some("mastracode"));
    assert_eq!(identify_name("opencode"), Some("opencode"));
    assert_eq!(identify_name("opencode.exe"), Some("opencode"));
    assert_eq!(identify_name("opencode2"), Some("opencode"));
    assert_eq!(identify_name("opencode2.exe"), Some("opencode"));
    assert_eq!(identify_name("kimi"), Some("kimi"));
    assert_eq!(identify_name("Kimi Code"), Some("kimi"));
    assert_eq!(identify_name("kiro"), Some("kiro"));
    assert_eq!(identify_name("kiro-cli"), Some("kiro"));
    assert_eq!(identify_name("copilot"), Some("copilot"));
    assert_eq!(identify_name("ghcs"), Some("copilot"));
    assert_eq!(identify_name("grok"), Some("grok"));
    assert_eq!(identify_name("grok-build"), Some("grok"));
    assert_eq!(identify_name("hermes"), Some("hermes"));
    assert_eq!(identify_name("hermes-agent"), Some("hermes"));
    assert_eq!(identify_name("kilo"), Some("kilo"));
    assert_eq!(identify_name("kilo-code"), Some("kilo"));
    assert_eq!(identify_name("qwen"), Some("qwen"));
    assert_eq!(identify_name("Qwen Code"), Some("qwen"));
    assert_eq!(identify_name("letta"), Some("letta"));
    assert_eq!(identify_name("Letta Code"), Some("letta"));
    assert_eq!(identify_name("maki"), Some("maki"));
    assert_eq!(identify_name("muse"), Some("muse"));
    assert_eq!(identify_name("muse-code"), Some("muse"));
    assert_eq!(identify_name("muse-cli"), Some("muse"));
    assert_eq!(identify_name("muse-bin-0.1.0-R708.1"), Some("muse"));
    assert_eq!(identify_name("muse-bin-1.2.3"), Some("muse"));
    assert_eq!(
        identify_name("/home/user/.local/bin/muse-bin-0.2.1-R1215.1"),
        Some("muse")
    );
    assert_eq!(
        identify_name(r"C:\Users\user\muse-bin-0.2.1-R1215.1.exe"),
        Some("muse")
    );
}

#[test]
fn identify_unknown_processes() {
    assert_eq!(identify_name("bash"), None);
    assert_eq!(identify_name("zsh"), None);
    assert_eq!(identify_name("vim"), None);
    assert_eq!(identify_name("node"), None);
    assert_eq!(identify_name("museum"), None);
    assert_eq!(identify_name("muse-helper"), None);
    assert_eq!(identify_name("muser"), None);
    assert_eq!(identify_name("musescore"), None);
    assert_eq!(identify_name("muse-bin"), None);
    assert_eq!(identify_name("muse-bin-"), None);
    assert_eq!(identify_name("muse-binary"), None);
}

#[test]
fn identify_case_insensitive() {
    assert_eq!(identify_name("Pi"), Some("pi"));
    assert_eq!(identify_name("CLAUDE"), Some("claude"));
    assert_eq!(identify_name("Codex"), Some("codex"));
    assert_eq!(identify_name("Devin"), Some("devin"));
}

#[test]
fn identify_agent_in_job_prefers_wrapped_codex() {
    let job = Job {
        process_group_id: 123,
        processes: vec![
            foreground_process(1, "node", &["node", "/path/to/bin/codex"]),
            foreground_process(2, "bash", &["bash"]),
        ],
    };

    assert_eq!(identify_in_job(&job), Some("codex".to_string()));
}

#[test]
fn identify_agent_in_job_detects_node_wrapped_qwen() {
    for argv in [
        vec!["node", "/home/user/.fnm/bin/qwen"],
        vec![
            "node.exe",
            r"C:\Users\user\AppData\Roaming\npm\node_modules\@qwen-code\qwen-code\dist\index.js",
        ],
    ] {
        let job = Job {
            process_group_id: 123,
            processes: vec![foreground_process(123, "MainThread", &argv)],
        };

        assert_eq!(identify_in_job(&job), Some("qwen".to_string()));
    }
}

#[test]
fn identify_agent_in_job_detects_cline_native_binaries() {
    for (name, executable) in [
        (
            ".cline",
            "/home/user/.npm/lib/node_modules/cline/bin/.cline",
        ),
        (
            "cline",
            "/usr/local/lib/node_modules/@cline/cli-darwin-arm64/bin/cline",
        ),
        (
            "cline.exe",
            r"C:\Users\user\AppData\Roaming\npm\node_modules\@cline\cli-windows-x64\bin\cline.exe",
        ),
    ] {
        let job = Job {
            process_group_id: 123,
            processes: vec![foreground_process(123, name, &[executable, "--tui"])],
        };

        assert_eq!(identify_in_job(&job), Some("cline".to_string()));
    }
}

#[test]
fn identify_agent_in_job_detects_cline_node_wrapper() {
    for (name, argv) in [
        (
            "MainThread",
            vec!["node", "/home/user/.fnm/bin/cline", "--tui"],
        ),
        (
            "node",
            vec!["node", "/usr/local/lib/node_modules/cline/bin/cline"],
        ),
        (
            "node.exe",
            vec![
                r"C:\Program Files\nodejs\node.exe",
                r"C:\Users\user\AppData\Roaming\npm\node_modules\cline\bin\cline",
            ],
        ),
    ] {
        let job = Job {
            process_group_id: 123,
            processes: vec![foreground_process(123, name, &argv)],
        };

        assert_eq!(identify_in_job(&job), Some("cline".to_string()));
    }
}

#[test]
fn identify_agent_in_job_rejects_unrelated_cline_mentions() {
    for argv in [
        vec!["node"],
        vec!["node", "/path/to/other.js", "cline"],
        vec!["node", "-e", "cline"],
        vec!["node", "/path/to/cline-helper"],
        vec!["/path/to/.cline-helper"],
        vec!["/path/to/other", "/path/to/cline"],
    ] {
        let job = Job {
            process_group_id: 123,
            processes: vec![foreground_process(123, "MainThread", &argv)],
        };

        assert_eq!(identify_in_job(&job), None);
    }
    assert_eq!(identify_name("MainThread"), None);
}

#[test]
fn identify_agent_in_job_detects_interactive_letta_entrypoints() {
    for argv in [
        vec!["letta", "--backend", "local"],
        vec![
            "node",
            "/home/user/project/node_modules/.bin/letta",
            "--conversation",
            "conversation-id",
        ],
        vec![
            "node.exe",
            r"C:\Users\user\AppData\Roaming\npm\node_modules\@letta-ai\letta-code\letta.js",
            "--agent",
            "agent-id",
        ],
    ] {
        let job = Job {
            process_group_id: 123,
            processes: vec![foreground_process(123, "MainThread", &argv)],
        };

        assert_eq!(identify_in_job(&job), Some("letta".to_string()));
    }
}

#[test]
fn identify_agent_in_job_ignores_noninteractive_letta_processes() {
    for args in [
        vec!["--prompt", "hello"],
        vec!["--output-format", "json"],
        vec!["--input-format=stream-json"],
        vec!["--ephemeral"],
        vec!["--max-turns=1"],
        vec!["server"],
        vec!["--backend", "local", "server"],
        vec!["fix this bug"],
        vec!["agents", "list"],
        vec!["version"],
    ] {
        let mut argv = vec!["node", "/home/user/project/node_modules/.bin/letta"];
        argv.extend(args);
        let job = Job {
            process_group_id: 123,
            processes: vec![foreground_process(123, "MainThread", &argv)],
        };

        assert_eq!(identify_in_job(&job), None, "argv: {argv:?}");
    }

    let unrelated = Job {
        process_group_id: 123,
        processes: vec![foreground_process(
            123,
            "node",
            &["node", "/tmp/server.js", "letta"],
        )],
    };
    assert_eq!(identify_in_job(&unrelated), None);

    let source_checkout = Job {
        process_group_id: 123,
        processes: vec![foreground_process(
            123,
            "node",
            &["node", "/home/user/src/letta-code/letta/build.js"],
        )],
    };
    assert_eq!(identify_in_job(&source_checkout), None);
}

#[test]
fn identify_agent_in_job_detects_windows_cursor_install() {
    let job = Job {
        process_group_id: 123,
        processes: vec![foreground_process(
            123,
            "node.exe",
            &[
                r"C:\Users\user\AppData\Local\cursor-agent\versions\2026.08.11-e8db854\node.exe",
                r"C:\Users\user\AppData\Local\cursor-agent\versions\2026.08.11-e8db854\index.js",
            ],
        )],
    };

    assert_eq!(identify_in_job(&job), Some("cursor".to_string()));
}

#[test]
fn identify_agent_in_job_ignores_invalid_windows_cursor_install_paths() {
    for script in [
        r"C:\Users\user\AppData\Local\cursor-agent\versions\2026.08.11-e8db854\scripts\postinstall.js",
        r"C:\Users\user\AppData\Local\cursor-agent\versions\2026.08.11-e8db854\index",
        r"C:\Users\user\AppData\Local\cursor-agent\versions\2026.08.11-e8db854\index.exe",
    ] {
        let job = Job {
            process_group_id: 123,
            processes: vec![foreground_process(
                123,
                "node.exe",
                &[
                    r"C:\Users\user\AppData\Local\cursor-agent\versions\2026.08.11-e8db854\node.exe",
                    script,
                ],
            )],
        };

        assert_eq!(identify_in_job(&job), None, "script: {script}");
    }

    let lookalike = Job {
        process_group_id: 123,
        processes: vec![foreground_process(
            123,
            "node.exe",
            &[
                r"C:\Program Files\nodejs\node.exe",
                r"C:\workspace\cursor-agent\versions\test\index.js",
            ],
        )],
    };
    assert_eq!(identify_in_job(&lookalike), None);
}

#[test]
fn identify_agent_in_job_prefers_recognized_process_group_leader() {
    let job = Job {
        process_group_id: 42,
        processes: vec![
            foreground_process(42, "claude", &["claude"]),
            foreground_process(43, "node", &["node", "/tmp/mcp/bin/codex"]),
        ],
    };

    assert_eq!(identify_in_job(&job), Some("claude".to_string()));
}

#[test]
fn identify_agent_in_job_falls_back_when_process_group_leader_is_unrecognized() {
    let job = Job {
        process_group_id: 42,
        processes: vec![
            foreground_process(42, "bash", &["bash"]),
            foreground_process(43, "node", &["node", "/tmp/mcp/bin/codex"]),
        ],
    };

    assert_eq!(identify_in_job(&job), Some("codex".to_string()));
}

#[test]
fn identify_agent_in_job_detects_python_version_wrapped_hermes() {
    let job = Job {
        process_group_id: 123,
        processes: vec![foreground_process(
            123,
            "python3.12",
            &[
                "/nix/store/example/bin/python3.12",
                "/nix/store/example/bin/hermes",
                "--resume",
                "session-id",
            ],
        )],
    };

    assert_eq!(identify_in_job(&job), Some("hermes".to_string()));
}

#[test]
fn identify_agent_in_job_detects_nix_wrapped_codex_from_cmdline_argv0() {
    let job = Job {
        process_group_id: 123,
        processes: vec![foreground_process(
            1,
            ".codex-wrapped",
            &["/etc/profiles/per-user/user/bin/codex", "--model", "gpt-5"],
        )],
    };

    assert_eq!(identify_in_job(&job), Some("codex".to_string()));
}

#[test]
fn identify_agent_in_job_canonicalizes_nix_wrapped_aliases_from_cmdline_argv0() {
    let job = Job {
        process_group_id: 123,
        processes: vec![foreground_process(
            1,
            ".claude-code-wrapped",
            &["/nix/store/example/bin/claude-code"],
        )],
    };

    assert_eq!(identify_in_job(&job), Some("claude".to_string()));
}

#[test]
fn identify_agent_in_job_detects_shell_wrapped_pi() {
    let job = Job {
        process_group_id: 123,
        processes: vec![foreground_process(
            1,
            "sh",
            &["/bin/sh", "/tmp/test-bin/pi"],
        )],
    };

    assert_eq!(identify_in_job(&job), Some("pi".to_string()));
}

#[test]
fn identify_agent_in_job_detects_bun_wrapped_omp() {
    for (runtime, script) in [
        ("bun", "/home/can/.bun/bin/omp"),
        (
            "bun.exe",
            r"C:\Users\herdr\AppData\Roaming\npm\node_modules\@oh-my-pi\pi-coding-agent\dist\cli.js",
        ),
    ] {
        let job = Job {
            process_group_id: 123,
            processes: vec![foreground_process(123, runtime, &[runtime, script])],
        };
        assert_eq!(
            identify_in_job(&job),
            Some("omp".to_string()),
            "script: {script}"
        );
    }

    let other_script = Job {
        process_group_id: 123,
        processes: vec![foreground_process(
            123,
            "bun.exe",
            &[
                "bun.exe",
                r"C:\Users\herdr\AppData\Roaming\npm\node_modules\@oh-my-pi\pi-coding-agent\dist\setup.js",
            ],
        )],
    };
    assert_eq!(identify_in_job(&other_script), None);
}

#[test]
fn identify_agent_in_job_detects_node_wrapped_pi_package_cli() {
    let job = Job {
        process_group_id: 123,
        processes: vec![foreground_process(
            123,
            "node.exe",
            &[
                "node.exe",
                "C:\\Users\\herdr\\AppData\\Roaming\\npm\\node_modules\\@earendil-works\\pi-coding-agent\\dist\\cli.js",
            ],
        )],
    };

    assert_eq!(identify_in_job(&job), Some("pi".to_string()));
}

#[test]
fn identify_agent_in_job_detects_node_wrapped_pi_bundled_cli() {
    let job = Job {
        process_group_id: 123,
        processes: vec![foreground_process(
            123,
            "node.exe",
            &[
                r"C:\Users\herdr\AppData\Local\pi-node\current\node.exe",
                r"C:\Users\herdr\AppData\Local\pi-node\current/node_modules/@earendil-works/pi-coding-agent/dist/bundle/cli.js",
            ],
        )],
    };

    assert_eq!(identify_in_job(&job), Some("pi".to_string()));
}

#[test]
fn identify_agent_in_job_detects_node_wrapped_mastracode_package_cli() {
    let job = Job {
        process_group_id: 123,
        processes: vec![foreground_process(
            123,
            "node.exe",
            &[
                "node.exe",
                "C:\\Users\\herdr\\AppData\\Roaming\\npm\\node_modules\\mastracode\\dist\\cli.js",
            ],
        )],
    };

    assert_eq!(identify_in_job(&job), Some("mastracode".to_string()));
}

#[test]
fn identify_agent_in_job_detects_node_wrapped_kimi_package_cli() {
    let job = Job {
        process_group_id: 123,
        processes: vec![foreground_process(
            123,
            "node.exe",
            &[
                r"C:\Program Files\nodejs\node.exe",
                r"C:\repro-3317-kimi-prefix\node_modules\@moonshot-ai\kimi-code\dist\main.mjs",
            ],
        )],
    };

    assert_eq!(identify_in_job(&job), Some("kimi".to_string()));
}

#[test]
fn identify_agent_in_job_ignores_non_cli_pi_package_scripts() {
    for script in [
        r"C:\Users\herdr\AppData\Roaming\npm\node_modules\@earendil-works\pi-coding-agent\scripts\build.js",
        r"C:\Users\herdr\AppData\Local\pi-node\current\node_modules\@earendil-works\pi-coding-agent\dist\bundle\update.js",
        r"C:\workspace\dist\bundle\cli.js",
        r"C:\workspace\node_modules\other-package\dist\bundle\cli.js",
        r"C:\workspace\node_modules\@earendil-works\pi-coding-agent\dist\cli.exe",
        r"C:\workspace\node_modules\@earendil-works\pi-coding-agent\dist\cli.js\other.js",
        r"C:\workspace\node_modules\@earendil-works\pi-coding-agent\dist\bundle\cli.exe",
        r"C:\workspace\node_modules\@earendil-works\pi-coding-agent\dist\bundle\cli.js\other.js",
    ] {
        let job = Job {
            process_group_id: 123,
            processes: vec![foreground_process(123, "node.exe", &["node.exe", script])],
        };

        assert_eq!(identify_in_job(&job), None, "script: {script}");
    }
}

#[test]
fn identify_agent_in_job_detects_windows_cmd_wrapped_codex() {
    let job = Job {
        process_group_id: 123,
        processes: vec![foreground_process(
            1,
            "cmd.exe",
            &[
                "cmd.exe",
                "/D",
                "/S",
                "/C",
                "C:\\Users\\herdr\\AppData\\Roaming\\npm\\codex.cmd --model gpt-5",
            ],
        )],
    };

    assert_eq!(identify_in_job(&job), Some("codex".to_string()));
}

#[test]
fn identify_agent_in_job_detects_powershell_file_wrapped_claude() {
    let job = Job {
        process_group_id: 123,
        processes: vec![foreground_process(
            1,
            "powershell.exe",
            &[
                "powershell.exe",
                "-NoProfile",
                "-File",
                "C:\\Users\\herdr\\Documents\\PowerShell\\Scripts\\claude.ps1",
            ],
        )],
    };

    assert_eq!(identify_in_job(&job), Some("claude".to_string()));
}

// A plain shell pane launched with herdr's injected prompt integration
// must still classify as a shell, not an agent, even though its argv now
// carries a -Command payload.
#[test]
fn identify_agent_in_job_ignores_herdr_powershell_shell_integration_argv() {
    let job = Job {
        process_group_id: 123,
        processes: vec![foreground_process(
            1,
            "powershell.exe",
            &[
                "powershell.exe",
                "-NoExit",
                "-Command",
                WINDOWS_POWERSHELL_SHELL_INTEGRATION_COMMAND,
            ],
        )],
    };

    assert_eq!(identify_in_job(&job), None);
}

#[test]
fn identify_agent_in_job_detects_opencode2_as_opencode() {
    let job = Job {
        process_group_id: 123,
        processes: vec![foreground_process(
            123,
            "opencode2",
            &["opencode2", "--standalone"],
        )],
    };

    assert_eq!(identify_in_job(&job), Some("opencode".to_string()));
}

#[test]
fn identify_agent_in_job_detects_opencode_exe_from_pnpm_package() {
    let job = Job {
        process_group_id: 123,
        processes: vec![foreground_process(
            123,
            "opencode.exe",
            &["/home/user/.local/share/pnpm/global/node_modules/opencode-ai/bin/opencode.exe"],
        )],
    };

    assert_eq!(identify_in_job(&job), Some("opencode".to_string()));
}

#[test]
fn identify_agent_in_job_detects_opencode_exe_from_argv0_path() {
    let job = Job {
        process_group_id: 123,
        processes: vec![foreground_process(
            123,
            "MainThread",
            &["/home/user/.local/share/pnpm/global/node_modules/opencode-ai/bin/opencode.exe"],
        )],
    };

    assert_eq!(identify_in_job(&job), Some("opencode".to_string()));
}

#[test]
fn wrapped_agent_name_from_runtime_argv_ignores_plain_shell_flags() {
    let job = Job {
        process_group_id: 1,
        processes: vec![foreground_process(1, "bash", &["bash", "-lc"])],
    };
    assert_eq!(identify_in_job(&job), None);
}

#[test]
fn identify_agent_in_job_ignores_python_c_argument_named_codex() {
    let job = Job {
        process_group_id: 123,
        processes: vec![foreground_process(
            1,
            "python3",
            &["python3", "-c", "import time; time.sleep(60)", "/tmp/codex"],
        )],
    };

    assert_eq!(identify_in_job(&job), None);
}

#[test]
fn identify_agent_in_job_ignores_node_eval_argument_named_codex() {
    let job = Job {
        process_group_id: 123,
        processes: vec![foreground_process(
            1,
            "node",
            &["node", "-e", "setTimeout(() => {}, 60000)", "/tmp/codex"],
        )],
    };

    assert_eq!(identify_in_job(&job), None);
}

#[test]
fn identify_agent_in_job_ignores_shell_c_argument_named_codex() {
    let job = Job {
        process_group_id: 123,
        processes: vec![foreground_process(
            1,
            "bash",
            &["bash", "-c", "sleep 60", "/tmp/codex"],
        )],
    };

    assert_eq!(identify_in_job(&job), None);
}

#[test]
fn identify_agent_in_job_detects_python_script_named_codex() {
    let job = Job {
        process_group_id: 123,
        processes: vec![foreground_process(
            1,
            "python3",
            &["python3", "/tmp/codex", "--model", "gpt-5"],
        )],
    };

    assert_eq!(identify_in_job(&job), Some("codex".to_string()));
}

#[test]
fn cmdline_argv0_agent_name_canonicalizes_known_aliases() {
    assert_eq!(
        identify_in_job(&argv0_only("/nix/store/example/bin/ghcs")),
        Some("copilot".to_string())
    );
}

#[test]
fn cmdline_argv0_agent_name_requires_exact_agent_basename() {
    assert_eq!(identify_in_job(&argv0_only("/tmp/my-codex-helper")), None);
}

#[cfg(unix)]
/// The walk over `/proc`, against a real job: a script named after Codex, run by `sh` in a
/// process group of its own, with a `sleep` under it. The leader's `comm` says `sh`; its argv is
/// what names the agent, and the child is in the job too.
/// Not herdr's: Gemini as npm installs it, a `node` whose main thread has renamed the process.
#[test]
fn a_node_named_after_its_main_thread_is_read_as_node() {
    let job = Job {
        process_group_id: 123,
        processes: vec![
            foreground_process(
                123,
                "node-MainThread",
                &["node", "/home/user/.npm-global/bin/gemini"],
            ),
            foreground_process(
                124,
                "node-MainThread",
                &[
                    "/usr/bin/node",
                    "--max-old-space-size=15848",
                    "/home/user/.npm-global/bin/gemini",
                ],
            ),
        ],
    };
    assert_eq!(identify_in_job(&job), Some("gemini".to_string()));

    let other = Job {
        process_group_id: 123,
        processes: vec![foreground_process(
            123,
            "node-MainThread",
            &["node", "/srv/app/server.js"],
        )],
    };
    assert_eq!(identify_in_job(&other), None);
}
