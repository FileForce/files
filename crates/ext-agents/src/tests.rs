use super::*;
use crate::stream::{Event, parse_line};

fn scratch(name: &str) -> PathBuf {
    files_sdk::permissions::declare(&MANIFEST);
    let dir = std::env::temp_dir().join(format!("files-ext-agents-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn roots(dir: &Path) -> Roots {
    Roots {
        claude: dir.join("claude"),
        claude_account: dir.join("claude.json"),
        codex: dir.join("codex"),
        opencode_db: dir.join("opencode.db"),
    }
}

const CLAUDE_LOG: &str = r#"{"type":"permission-mode","permissionMode":"default","sessionId":"s1"}
{"type":"user","isMeta":true,"message":{"role":"user","content":"<local-command-caveat>ignore</local-command-caveat>"},"timestamp":"2026-10-29T07:00:00.000Z","cwd":"/work/app","entrypoint":"cli"}
{"type":"user","message":{"role":"user","content":"Add a dark theme\nwith tokens"},"timestamp":"2026-10-29T07:47:27.473Z","cwd":"/work/app"}
{"type":"assistant","message":{"id":"msg_1","model":"claude-opus-5-5","content":[{"type":"text","text":"Sure, "}],"usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":100,"cache_creation_input_tokens":50,"output_tokens_details":{"thinking_tokens":2}}},"timestamp":"2026-10-29T07:47:30.000Z"}
{"type":"assistant","message":{"id":"msg_1","model":"claude-opus-5-5","content":[{"type":"text","text":"done."}],"usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":100,"cache_creation_input_tokens":50}},"timestamp":"2026-10-29T07:47:31.000Z"}
{"type":"assistant","message":{"id":"msg_2","model":"claude-opus-5-5","content":[{"type":"tool_use","name":"Bash","input":{"command":"cargo test"}}],"usage":{"input_tokens":1,"output_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}},"timestamp":"2026-10-29T07:48:00.000Z"}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"ok: 3 passed"}]},"timestamp":"2026-10-29T07:48:05.000Z"}
{"type":"ai-title","aiTitle":"Dark theme tokens","sessionId":"s1"}
"#;

const CODEX_LOG: &str = r#"{"timestamp":"2026-07-17T18:00:00.000Z","type":"session_meta","payload":{"id":"019f-codex","cwd":"/work/api","source":"cli","originator":"codex_cli_rs"}}
{"timestamp":"2026-07-17T18:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.5","cwd":"/work/api"}}
{"timestamp":"2026-07-17T18:00:02.000Z","type":"event_msg","payload":{"type":"user_message","message":"Why is the build slow?"}}
{"timestamp":"2026-07-17T18:00:05.000Z","type":"response_item","payload":{"type":"function_call","name":"shell"}}
{"timestamp":"2026-07-17T18:00:09.000Z","type":"event_msg","payload":{"type":"agent_message","message":"Incremental builds are off."}}
{"timestamp":"2026-07-17T18:00:10.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":800,"output_tokens":50,"reasoning_output_tokens":20,"total_tokens":1050}},"rate_limits":{"primary":{"used_percent":42.5,"window_minutes":300,"resets_at":1786903366},"secondary":{"used_percent":10.0,"window_minutes":10080,"resets_at":1787000000},"credits":{"has_credits":false,"unlimited":false,"balance":null},"plan_type":"plus","rate_limit_reached_type":null}}}
"#;

#[test]
fn claude_logs_give_title_usage_and_chat() {
    let dir = scratch("claude");
    let project = dir.join("claude/projects/-work-app");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("s1.jsonl"), CLAUDE_LOG).unwrap();
    // Only hooks, no prompt: not a session.
    std::fs::write(project.join("empty.jsonl"), "{\"type\":\"mode\",\"mode\":\"normal\"}\n").unwrap();

    let sessions = scan(&roots(&dir), &[]);
    assert_eq!(sessions.len(), 1, "{sessions:?}");
    let session = &sessions[0];
    assert_eq!((session.agent, session.id.as_str(), session.title.as_str()), (Agent::Claude, "s1", "Dark theme tokens"));
    assert_eq!(session.cwd.as_deref(), Some(Path::new("/work/app")));
    assert_eq!(session.model.as_deref(), Some("claude-opus-5-5"));
    assert_eq!(session.origin.as_deref(), Some("cli"));
    assert!(session.external);
    // msg_1 is logged twice but counted once.
    assert_eq!(
        (session.usage.input, session.usage.output, session.usage.cache_read, session.usage.cache_write, session.usage.reasoning),
        (11, 6, 100, 50, 2)
    );
    assert_eq!(session.updated, parse::iso_seconds("2026-10-29T07:48:05Z").unwrap());

    let chat = messages(&roots(&dir), session);
    let roles: Vec<_> = chat.iter().map(|message| message.role).collect();
    assert_eq!(roles, [Role::User, Role::Assistant, Role::Tool, Role::Tool]);
    assert_eq!(chat[1].text, "Sure, done.", "blocks of one message are joined");
    assert_eq!(chat[2].text, "⚙ Bash · cargo test");
    assert_eq!(chat[3].text, "↳ ok: 3 passed");

    // Ours, not external.
    assert!(!scan(&roots(&dir), &["s1".to_string()])[0].external);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn codex_logs_give_usage_limits_and_chat() {
    let dir = scratch("codex");
    let day = dir.join("codex/sessions/2026/07/17");
    std::fs::create_dir_all(&day).unwrap();
    std::fs::write(day.join("rollout-2026-07-17T18-00-00-019f-codex.jsonl"), CODEX_LOG).unwrap();

    let sessions = scan(&roots(&dir), &[]);
    let session = &sessions[0];
    assert_eq!((session.agent, session.id.as_str()), (Agent::Codex, "019f-codex"));
    assert_eq!(session.title, "Why is the build slow?");
    assert_eq!(session.model.as_deref(), Some("gpt-5.5"));
    assert_eq!((session.usage.input, session.usage.cache_read, session.usage.output, session.usage.reasoning), (200, 800, 50, 20));

    let chat = messages(&roots(&dir), session);
    assert_eq!(chat.iter().map(|message| message.role).collect::<Vec<_>>(), [Role::User, Role::Tool, Role::Assistant]);

    let accounts = accounts(&roots(&dir), &sessions);
    let (_, codex) = accounts.iter().find(|(agent, _)| *agent == Agent::Codex).unwrap();
    assert_eq!(codex.plan.as_deref(), Some("plus"));
    assert_eq!(codex.limits.len(), 2);
    assert_eq!((codex.limits[0].name.as_str(), codex.limits[0].used_percent), ("5 hours", 42.5));
    assert_eq!(codex.limits[1].name, "7 days");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn claude_account_has_plan_but_no_personal_data() {
    let dir = scratch("account");
    std::fs::write(
        dir.join("claude.json"),
        r#"{"oauthAccount":{"emailAddress":"me@example.com","accountUuid":"x","organizationName":"Acme","organizationType":"claude_enterprise","seatTier":"enterprise_usage_based","billingType":"aws_marketplace","hasExtraUsageEnabled":true}}"#,
    )
    .unwrap();
    let accounts = accounts(&roots(&dir), &[]);
    let (agent, account) = &accounts[0];
    assert_eq!(*agent, Agent::Claude);
    assert_eq!(account.plan.as_deref(), Some("claude enterprise"));
    assert_eq!(account.organization.as_deref(), Some("Acme"));
    let shown = format!("{account:?}");
    assert!(!shown.contains("me@example.com") && !shown.contains("accountUuid"), "{shown}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn opencode_database_is_read_only_and_complete() {
    let dir = scratch("opencode");
    let db = dir.join("opencode.db");
    let status = Command::new("sqlite3")
        .arg(&db)
        .arg(
            "create table session (id text, title text, directory text, model text, cost real, tokens_input integer, \
             tokens_output integer, tokens_reasoning integer, tokens_cache_read integer, tokens_cache_write integer, \
             time_updated integer, time_archived integer, parent_id text);
             create table message (id text, data text);
             create table part (id text, message_id text, session_id text, time_created integer, data text);
             insert into session values ('ses_1','Fix login','/work/web','{\"providerID\":\"x\",\"modelID\":\"k3\"}',0.25,100,20,5,300,0,1786262616000,null,null);
             insert into session values ('ses_old','Archived','/work/web',null,0,0,0,0,0,0,1,5,null);
             insert into message values ('m1','{\"role\":\"user\"}'), ('m2','{\"role\":\"assistant\"}');
             insert into part values ('p1','m1','ses_1',1,'{\"type\":\"text\",\"text\":\"2+2\"}');
             insert into part values ('p2','m2','ses_1',2,'{\"type\":\"text\",\"text\":\"4\"}');",
        )
        .status();
    if !status.is_ok_and(|status| status.success()) {
        eprintln!("sqlite3 not available; skipping");
        return;
    }
    let sessions = scan(&roots(&dir), &[]);
    assert_eq!(sessions.len(), 1, "archived sessions are hidden");
    let session = &sessions[0];
    assert_eq!((session.title.as_str(), session.model.as_deref()), ("Fix login", Some("k3")));
    assert_eq!(session.usage.cost, Some(0.25));
    assert_eq!((session.usage.input, session.usage.cache_read), (100, 300));
    let chat = messages(&roots(&dir), session);
    assert_eq!(chat, [ChatMessage { role: Role::User, text: "2+2".into() }, ChatMessage { role: Role::Assistant, text: "4".into() }]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn streams_of_all_three_clis() {
    let claude = [
        r#"{"type":"system","subtype":"init","session_id":"abc"}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Hel"}}}"#,
        r#"{"type":"result","is_error":false,"usage":{"input_tokens":3,"output_tokens":2},"total_cost_usd":0.01}"#,
    ];
    let events: Vec<_> = claude.iter().flat_map(|line| parse_line(Agent::Claude, line)).collect();
    assert_eq!(events[0], Event::Session("abc".into()));
    assert_eq!(events[1], Event::Text("Hel".into()));
    assert!(matches!(&events[2], Event::Done { usage } if usage.input == 3 && usage.cost == Some(0.01)));

    let codex = [
        r#"{"type":"thread.started","thread_id":"t1"}"#,
        r#"{"type":"item.completed","item":{"id":"i","type":"command_execution","command":"ls"}}"#,
        r#"{"type":"item.completed","item":{"id":"i","type":"agent_message","text":"Done"}}"#,
        r#"{"type":"turn.completed","usage":{"input_tokens":10,"cached_input_tokens":4,"output_tokens":2}}"#,
    ];
    let events: Vec<_> = codex.iter().flat_map(|line| parse_line(Agent::Codex, line)).collect();
    assert_eq!(events[..3], [Event::Session("t1".into()), Event::Tool("ls".into()), Event::Message("Done".into())]);
    assert!(matches!(&events[3], Event::Done { usage } if usage.input == 6 && usage.cache_read == 4));

    let opencode = r#"{"type":"text","sessionID":"ses_9","part":{"type":"text","text":"Hi"}}"#;
    assert_eq!(parse_line(Agent::OpenCode, opencode), [Event::Session("ses_9".into()), Event::Message("Hi".into())]);
    assert!(parse_line(Agent::Claude, "not json").is_empty());
}

#[test]
fn models_and_effort_flags() {
    let dir = scratch("models");
    std::fs::create_dir_all(dir.join("codex")).unwrap();
    std::fs::write(dir.join("codex/config.toml"), "model = \"gpt-5.5\"\nmodel_reasoning_effort = \"high\"\n").unwrap();
    std::fs::write(
        dir.join("codex/models_cache.json"),
        r#"{"models":[
            {"slug":"gpt-5.5","display_name":"GPT-5.5","default_reasoning_level":"medium","supported_reasoning_levels":[{"effort":"low"},{"effort":"xhigh"}],"visibility":"list"},
            {"slug":"internal","display_name":"Hidden","visibility":"hide"}]}"#,
    )
    .unwrap();
    let codex = models(&roots(&dir), Agent::Codex, &[]);
    assert_eq!(codex.len(), 1, "hidden models are not offered");
    assert_eq!((codex[0].label.as_str(), codex[0].is_default), ("GPT-5.5", true));
    assert_eq!(codex[0].efforts, ["low", "xhigh"]);
    assert_eq!(codex[0].default_effort.as_deref(), Some("high"), "config.toml wins");

    let used = Session {
        agent: Agent::Claude,
        id: "x".into(),
        title: "t".into(),
        cwd: None,
        model: Some("claude-opus-5-5".into()),
        updated: 0,
        usage: Usage::default(),
        origin: None,
        running: false,
        external: true,
        source: None,
    };
    let claude = models(&roots(&dir), Agent::Claude, &[used]);
    assert_eq!(claude.iter().map(|choice| choice.id.as_str()).collect::<Vec<_>>(), ["fable", "opus", "sonnet", "haiku", "claude-opus-5-5"]);
    assert_eq!(claude[0].efforts.last().map(String::as_str), Some("max"));

    let options = RunOptions { model: Some("opus".into()), effort: Some("high".into()) };
    assert_eq!(option_args(Agent::Claude, &options), ["--model", "opus", "--effort", "high"]);
    assert_eq!(option_args(Agent::Codex, &options), ["-m", "opus", "-c", "model_reasoning_effort=\"high\""]);
    assert_eq!(option_args(Agent::OpenCode, &options), ["--model", "opus", "--variant", "high"]);
    assert!(option_args(Agent::Codex, &RunOptions::default()).is_empty(), "nothing picked: the agent's own settings");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn processes_and_dates() {
    assert_eq!(agent_of_command("/opt/homebrew/bin/claude --resume x"), Some(Agent::Claude));
    assert_eq!(agent_of_command("node /Users/me/Library/pnpm/codex exec"), Some(Agent::Codex));
    assert_eq!(agent_of_command("/Users/me/.opencode/bin/opencode"), Some(Agent::OpenCode));
    assert_eq!(agent_of_command("/usr/bin/vim claude.md"), None);
    files_sdk::permissions::declare(&MANIFEST);
    use_program(Agent::OpenCode, PathBuf::from("/opt/it's/opencode"));
    assert_eq!(
        terminal_command(Agent::OpenCode, Path::new("/work/my app")).unwrap(),
        r"cd '/work/my app' && exec '/opt/it'\''s/opencode'"
    );
    assert_eq!(Agent::Codex.desktop_bundle(), "com.openai.codex");
    assert_eq!(parse::iso_seconds("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(parse::iso_seconds("2026-10-29T07:47:27.473Z"), Some(1_793_260_047));
}

#[test]
#[ignore = "reads this machine's real agent data"]
fn real_data_summary() {
    files_sdk::permissions::declare(&MANIFEST);
    let roots = Roots::detect();
    let started = std::time::Instant::now();
    let sessions = scan(&roots, &[]);
    for agent in Agent::ALL {
        let mine: Vec<_> = sessions.iter().filter(|session| session.agent == agent).collect();
        let mut total = Usage::default();
        mine.iter().for_each(|session| total.add(&session.usage));
        let running = mine.iter().filter(|session| session.running).count();
        let models: std::collections::BTreeSet<_> = mine.iter().filter_map(|session| session.model.clone()).collect();
        println!("{}: {} sessions, {running} running, models {models:?}, in {} out {} cache r/w {}/{} cost {:?}",
            agent.name(), mine.len(), total.input, total.output, total.cache_read, total.cache_write, total.cost);
        if let Some(first) = mine.first() {
            println!("   newest chat: {} messages", messages(&roots, first).len());
        }
    }
    for (agent, account) in accounts(&roots, &sessions) {
        println!("{}: plan {:?}, limits {:?}", agent.name(), account.plan, account.limits);
    }
    println!("scan took {:?}", started.elapsed());
    let again = std::time::Instant::now();
    scan(&roots, &[]);
    println!("cached rescan {:?}", again.elapsed());
}
