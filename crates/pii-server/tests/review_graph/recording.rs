//! Spec 403 (the recording provider and the tests that read what reached the agent): split out of `review_graph.rs`, which had grown past the file-size limit.

#![allow(clippy::wildcard_imports)] // reason: the shared fakes and helpers live in the parent module

use super::*;

/// Records every request it receives and answers with a pass.
#[derive(Default)]
struct Recording {
    seen: Mutex<Vec<ChatRequest>>,
}

impl ModelProvider for Recording {
    fn name(&self) -> &'static str {
        "recording"
    }

    fn chat(&self, _ctx: &Ctx, req: &ChatRequest) -> Result<ChatResponse, PiiError> {
        self.seen.lock().push(req.clone());
        Ok(ChatResponse {
            text: String::from("VERDICT: PASS"),
            tool_calls: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
            micro_usd: 0,
        })
    }
}

#[tokio::test]
async fn the_rubric_text_reaches_the_agent_and_is_not_in_the_graph() {
    let rubric_file = repo().join("rubrics/readiness.md");
    let rubric_text = std::fs::read_to_string(&rubric_file)
        .unwrap_or_else(|e| panic!("{}: {e}", rubric_file.display()));

    // The reviewer's config is the agent node as the graph declares it.
    let review_src = graph("review-one");
    let doc = table(&review_src);
    let agents = nodes_of_kind(&doc, "agent");
    assert_eq!(agents.len(), 1, "one reviewer node");
    let cfg = serde_json::to_value(agents[0]).unwrap();
    assert_eq!(cfg["system"], SYSTEM, "the system is the one-line literal");

    let recording = Arc::new(Recording::default());
    let factory = AgentFactory {
        build_provider: Arc::new(pii_model::provider_from_spec),
        provider: recording.clone(),
        route_policy: BTreeMap::new(),
        workspace: PathBuf::from(WORKDIR),
        live: None,
        memory: None,
        tiers: None,
        models: Arc::default(),
    };
    let node = factory.build(&cfg).expect("the reviewer builds");
    let input = serde_json::json!({
        "doc": DOC,
        "rubric": "readiness",
        "workdir": WORKDIR,
        "model": "@planner",
        "provider": "scripted",
    });
    let node_input = NodeInput {
        input: input.clone(),
        payload: input,
        nodes: serde_json::json!({"rubric": {"stdout": rubric_text}}),
    };
    let ctx = Ctx::root(CapSet::empty(), Budget::unlimited());
    node.run(&ctx, &node_input).expect("the reviewer runs");

    let prompt: String = recording
        .seen
        .lock()
        .iter()
        .flat_map(|req| req.messages.iter())
        .map(|m| m.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        prompt.contains(rubric_text.trim()),
        "the prompt carries the rubric file's bytes"
    );
    assert!(prompt.contains(DOC), "the prompt carries the document path");
    assert!(!prompt.contains("@planner"), "no model name in the prompt");
    assert!(!prompt.contains(WORKDIR), "no workdir in the prompt");

    for src in [review_src, graph("review")] {
        for entry in std::fs::read_dir(repo().join("rubrics")).unwrap() {
            let text = std::fs::read_to_string(entry.unwrap().path()).unwrap();
            for line in text.lines().map(str::trim).filter(|l| l.len() >= 30) {
                assert!(!src.contains(line), "a graph holds rubric text: {line}");
            }
        }
    }
}

#[tokio::test]
async fn a_rubric_added_as_a_file_is_a_reviewer_with_no_other_change() {
    let before = (
        std::fs::read(repo().join("graphs/review-one.toml")).unwrap(),
        std::fs::read(repo().join("graphs/review.toml")).unwrap(),
    );
    let (_dir, wd) = workdir(&[("probe-403", RUBRIC_BODY)]);
    let mut input = base(&wd);
    input["rubric"] = serde_json::json!("probe-403");
    let settled = run("review-one", &["VERDICT: PASS"], &input).await;
    assert!(settled.passed(), "{:?}", settled.text());
    let after = (
        std::fs::read(repo().join("graphs/review-one.toml")).unwrap(),
        std::fs::read(repo().join("graphs/review.toml")).unwrap(),
    );
    assert!(before == after, "the graph files are byte-identical");
}
