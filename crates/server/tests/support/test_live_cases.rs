//! Explicit CLI fixture suite. Default check never enables or invokes a live
//! provider command. Run with a separately built job-failpoints xtask binary.
use super::*;
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};

struct LiveFiles {
    _output: TestDir,
    case: PathBuf,
    budget: PathBuf,
    case_value: Value,
}
fn binary() -> PathBuf {
    PathBuf::from(
        std::env::var_os("EM_PC04_XTASK_BINARY")
            .expect("explicit isolated xtask fixture binary required"),
    )
}
fn command(files: &LiveFiles, plan: bool) -> Command {
    let mut cmd = Command::new(binary());
    cmd.env_clear()
        .env("PC04_FAKE_KEY", CANARY_KEY)
        .arg("test-live")
        .arg("--case")
        .arg(&files.case);
    if plan {
        cmd.arg("--plan");
    } else {
        cmd.arg("--budget-file").arg(&files.budget);
    }
    cmd
}
fn output_json(output: &Output) -> Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stdout.contains(CANARY_KEY) && !stderr.contains(CANARY_KEY),
        "secret output"
    );
    let index = stdout
        .find("\n{")
        .unwrap_or_else(|| panic!("missing JSON: {stdout} {stderr}"));
    serde_json::from_str(&stdout[index + 1..]).expect("whitelist JSON output")
}
fn private_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
}
async fn files(chain: &Chain) -> LiveFiles {
    let data = fs::canonicalize(chain.app.dir()).unwrap();
    let output = TestDir::new("pc04-output");
    let output_path = fs::canonicalize(output.path()).unwrap();
    let config = data.join("config.toml");
    let settings = chain.app.state().settings();
    fs::write(
        &config,
        format!(
            r#"price_catalog_path = "{}"
[providers.tripo]
base_url = "{}"
model = "v3.1-20260211"
api_key_env = "PC04_FAKE_KEY"
[providers.manual_ai]
base_url = "{}"
model = "gpt-5-mini"
api_key_env = "PC04_FAKE_KEY"
[download]
allowed_hosts = ["127.0.0.1"]
allow_local_fixture = true
[jobs]
lease_seconds = 3
renew_seconds = 1
"#,
            data.join("price-catalog.toml").display(),
            settings.providers.tripo.base_url,
            settings.providers.manual_ai.base_url
        ),
    )
    .unwrap();
    let mut conn = pool(chain).acquire().await.unwrap();
    let prep = repo::preparations::get(&mut conn, &chain.inputs.preparation)
        .await
        .unwrap()
        .unwrap();
    let photos = everything_manual::generation::estimate::load_photos_for_item(
        &mut conn,
        &chain.inputs.item,
        &chain.inputs.photo_ids,
    )
    .await
    .unwrap();
    let case = json!({"schemaVersion":1,"caseId":"fixture-only-pc04","mode":"loopbackFixture","instance":{"instanceId":"isolated-fixture","dataDir":data,"configFile":config},"material":{"itemId":chain.inputs.item,"itemModel":"X100V","documentId":prep.document_id,"sourceSha256":prep.source_sha256,"preparationId":prep.id,"photos":photos.iter().map(|p|json!({"id":p.photo.id,"sha256":p.sha256,"view":p.photo.view.as_str()})).collect::<Vec<_>>()},"generation":{"modelPreset":PRESET,"tripo":{"identity":"tripo","model":"v3.1-20260211"},"manualAi":{"identity":"manual_ai","model":MANUAL_AI_MODEL},"priceVersion":"2026-09-11"},"outputDirectory":output_path});
    let case_path = output.join("case.json");
    let budget = output.join("fixture-authorization.json");
    private_json(&case_path, &case);
    LiveFiles {
        _output: output,
        case: case_path,
        budget,
        case_value: case,
    }
}
fn budget_for(plan: &Value) -> Value {
    json!({"schemaVersion":1,"authorizationId":"fixture-only-initial-001","caseId":plan["caseId"],"caseHash":plan["caseHash"],"planHash":plan["planHash"],"allowed":true,"expiresAt":Timestamp::now().checked_add_millis(600000).unwrap(),"limits":plan["requiredLimits"],"maxInitialGenerations":1,"retryScopes":[]})
}
fn delay(step: Step, millis: u64) -> Step {
    let Step::Respond { response } = step else {
        panic!("response required")
    };
    Step::Delay {
        delay_ms: millis,
        response,
    }
}
fn spawn(files: &LiveFiles, failpoint: Option<&str>) -> std::process::Child {
    let mut cmd = command(files, false);
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(failpoint) = failpoint {
        cmd.env("EM_TEST_FAILPOINT", failpoint);
    }
    cmd.spawn().unwrap()
}
async fn wait(mut child: std::process::Child) -> Output {
    for _ in 0..600 {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    child.kill().unwrap();
    let out = child.wait_with_output().unwrap();
    panic!(
        "CLI fixture exceeded 30 seconds: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}
async fn wait_calls(server: &FixtureServer, path: &str, count: usize) {
    for _ in 0..200 {
        if server.call_count("POST", path) >= count {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("fixture call did not arrive")
}
async fn authorize_files(chain: &Chain) -> (LiveFiles, Value) {
    let f = files(chain).await;
    let p = output_json(&command(&f, true).output().unwrap());
    private_json(&f.budget, &budget_for(&p));
    (f, p)
}
async fn count(chain: &Chain, table: &str) -> i64 {
    sqlx::query_scalar(match table {
        "jobs" => "SELECT count(*) FROM jobs",
        "quotes" => "SELECT count(*) FROM quotes",
        "manual_releases" => "SELECT count(*) FROM manual_releases",
        _ => panic!("test count table must be explicit"),
    })
    .fetch_one(&pool(chain))
    .await
    .unwrap()
}

#[tokio::test]
#[ignore = "requires explicit job-failpoints xtask binary; localhost only"]
async fn pc04_cli_plan_full_fixture_and_same_authorization_replay() {
    let manual = manual_ai_server(vec![respond_file(&responses_path("success.json"))]);
    let (cdn, url) = cdn_server("/model.glb", fixture_bytes("sample-model.glb"));
    let tripo = tripo_server(vec![submit_success()], vec![task_success_with(&url)]);
    let chain = chain("pc04-full", &tripo, &manual).await;
    let files = files(&chain).await;
    let plan_output = command(&files, true).output().unwrap();
    assert!(
        plan_output.status.success(),
        "{}",
        String::from_utf8_lossy(&plan_output.stderr)
    );
    let plan = output_json(&plan_output);
    let repeated = output_json(&command(&files, true).output().unwrap());
    assert_eq!(plan, repeated);
    assert_eq!(plan["batches"], json!([[1, 2, 3]]));
    assert_eq!(plan["views"].as_array().unwrap().len(), 2);
    assert_eq!(
        tripo.request_total() + manual.request_total() + cdn.request_total(),
        0
    );
    assert_eq!(
        count(&chain, "quotes").await,
        0,
        "plan rolls its same-source quote back"
    );
    private_json(&files.budget, &budget_for(&plan));
    let executed = command(&files, false).output().unwrap();
    let result = output_json(&executed);
    assert!(
        executed.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&executed.stdout),
        String::from_utf8_lossy(&executed.stderr)
    );
    assert_eq!(result["status"], "needs_review");
    assert_eq!(result["AC-042"], "NOT_RUN");
    assert_eq!(count(&chain, "jobs").await, 1);
    assert_eq!(count(&chain, "manual_releases").await, 0);
    let submitted = tripo.requests_matching("POST", TRIPO_SUBMIT_PATH)[0].json_body();
    assert_eq!(
        submitted["inputs"],
        json!([{"front":"token-0-t19"},{"left":"token-1-t19"}])
    );
    assert_eq!(submitted["model"], "v3.1-20260211");
    let extracted = manual.requests_matching("POST", MANUAL_AI_PATH)[0].json_body();
    assert_eq!(extracted["model"], MANUAL_AI_MODEL);
    assert_eq!(extracted["text"]["format"]["strict"], true);
    assert_eq!(extracted["max_output_tokens"], plan["maxOutputTokens"]);
    let prompt = extracted["input"][0]["content"][0]["text"]
        .as_str()
        .unwrap();
    for page in 1..=3 {
        assert!(prompt.contains(&format!("[第 {page} 页]")));
    }
    assert!(!prompt.contains("[第 4 页]"));
    let calls = tripo.request_total() + manual.request_total() + cdn.request_total();
    assert_eq!(tripo.call_count("POST", TRIPO_SUBMIT_PATH), 1);
    assert_eq!(manual.call_count("POST", MANUAL_AI_PATH), 1);
    let replay = command(&files, false).output().unwrap();
    assert!(
        replay.status.success(),
        "{}",
        String::from_utf8_lossy(&replay.stderr)
    );
    let replay = output_json(&replay);
    assert_eq!(replay["jobId"], result["jobId"]);
    assert_eq!(
        calls,
        tripo.request_total() + manual.request_total() + cdn.request_total()
    );
    assert_eq!(count(&chain, "jobs").await, 1);
    assert_eq!(count(&chain, "quotes").await, 1);
    let dir = fs::read_dir(files._output.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .find(|e| e.file_type().unwrap().is_dir())
        .unwrap()
        .path();
    test_support::assets::validate_glb(&fs::read(dir.join("model.glb")).unwrap()).unwrap();
    for name in ["knowledge.json", "report.json"] {
        let text = fs::read_to_string(dir.join(name)).unwrap();
        assert!(!text.contains(CANARY_KEY));
        assert!(!text.contains("http://"));
        assert!(!text.contains(chain.app.dir().to_str().unwrap()));
    }
    let knowledge: Value =
        serde_json::from_slice(&fs::read(dir.join("knowledge.json")).unwrap()).unwrap();
    assert!(
        knowledge["knowledge"]["parts"][0]["evidence"][0]["pageNumber"]
            .as_i64()
            .is_some_and(|p| p >= 1)
    );
    tripo.assert_no_script_problems();
    manual.assert_no_script_problems();
    cdn.assert_no_script_problems();
    cdn.shutdown();
    tripo.shutdown();
    manual.shutdown();
}

#[tokio::test]
#[ignore = "requires explicit job-failpoints xtask binary; localhost only"]
async fn pc04_cli_preflight_rejections_are_safe_zero_call_and_zero_job() {
    let manual = manual_ai_server(vec![]);
    let tripo = tripo_server(vec![], vec![]);
    let chain = chain("pc04-reject", &tripo, &manual).await;
    let mut files = files(&chain).await;
    let help = Command::new(binary())
        .env_clear()
        .args(["test-live", "--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("--budget-file"));
    let missing = Command::new(binary())
        .env_clear()
        .arg("test-live")
        .arg("--case")
        .arg(&files.case)
        .output()
        .unwrap();
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("budgetRequired"));
    let plan = output_json(&command(&files, true).output().unwrap());
    let valid = budget_for(&plan);
    let mut variants = vec![];
    for (key, value) in [
        ("allowed", json!(false)),
        ("expiresAt", json!("2001-01-01T00:00:00Z")),
        ("authorizationId", json!("sk-proj-qa_fake_0123456789")),
        ("caseHash", json!("0".repeat(64))),
        ("planHash", json!("0".repeat(64))),
        ("maxInitialGenerations", json!(2)),
        ("apiKey", json!(CANARY_KEY)),
    ] {
        let mut b = valid.clone();
        b[key] = value;
        variants.push(b);
    }
    for key in ["creditMinor", "usdMicros"] {
        for value in [
            json!(0),
            json!(-1),
            json!(1.5),
            json!("3000"),
            serde_json::from_str("9223372036854775808").unwrap(),
        ] {
            let mut b = valid.clone();
            b["limits"][key] = value;
            variants.push(b);
        }
    }
    for variant in variants {
        private_json(&files.budget, &variant);
        let out = command(&files, false).output().unwrap();
        assert!(!out.status.success());
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!err.contains(CANARY_KEY) && !err.contains("sk-proj-qa_fake"));
        assert!(err.contains("未发起供应商请求"));
    }
    private_json(&files.budget, &valid);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&files.budget, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!command(&files, false).output().unwrap().status.success());
    }
    private_json(&files.budget, &valid);
    fs::write(&files.budget, "{secret-canary").unwrap();
    assert!(!command(&files, false).output().unwrap().status.success());
    private_json(&files.budget, &valid);
    let original_case = files.case_value.clone();
    for field in ["sourceSha256", "preparationId"] {
        files.case_value = original_case.clone();
        files.case_value["material"][field] = json!(if field == "sourceSha256" {
            "0".repeat(64)
        } else {
            "00000000-0000-0000-0000-000000000000".into()
        });
        private_json(&files.case, &files.case_value);
        assert!(!command(&files, false).output().unwrap().status.success());
    }
    files.case_value = original_case;
    private_json(&files.case, &files.case_value);
    let config_path = chain.app.dir().join("config.toml");
    let config = fs::read_to_string(&config_path).unwrap();
    for changed in [
        config.replace(
            "model = \"gpt-5-mini\"",
            "model = \"sk-proj-qa_fake_0123456789\"",
        ),
        config.replace(
            "api_key_env = \"PC04_FAKE_KEY\"",
            "api_key_env = \"ABSENT_FIXTURE_KEY\"",
        ),
    ] {
        fs::write(&config_path, changed).unwrap();
        let output = command(&files, false).output().unwrap();
        assert!(!output.status.success());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("sk-proj-qa_fake"));
    }
    fs::write(&config_path, config).unwrap();
    let catalog = chain.app.dir().join("price-catalog.toml");
    let original_catalog = fs::read_to_string(&catalog).unwrap();
    fs::write(&catalog, "version = \"no-prices\"\n").unwrap();
    assert!(!command(&files, false).output().unwrap().status.success());
    fs::write(&catalog, original_catalog).unwrap();
    files.case_value["generation"]["modelPreset"] = json!("unsupported-preset");
    private_json(&files.case, &files.case_value);
    assert!(!command(&files, false).output().unwrap().status.success());
    assert_eq!(count(&chain, "jobs").await, 0);
    assert_eq!(count(&chain, "quotes").await, 0);
    assert_eq!(tripo.request_total() + manual.request_total(), 0);
    tripo.shutdown();
    manual.shutdown();
}

#[tokio::test]
#[ignore = "requires explicit job-failpoints xtask binary; localhost only"]
async fn pc04_cli_concurrent_same_authorization_and_unrelated_expired_job_are_isolated() {
    let manual = manual_ai_server(vec![delay(
        respond_file(&responses_path("success.json")),
        800,
    )]);
    let (cdn, url) = cdn_server("/model.glb", fixture_bytes("sample-model.glb"));
    let tripo = tripo_server(vec![submit_success()], vec![task_success_with(&url)]);
    let chain = chain("pc04-concurrent", &tripo, &manual).await;
    let unrelated = create_job(
        &chain.app,
        &chain.cookie,
        &chain.csrf,
        &chain.inputs,
        "unrelated-web-job",
    )
    .await;
    sqlx::query("UPDATE job_stages SET status='running',lease_owner='unrelated-owner',lease_epoch=8,lease_until=1 WHERE job_id=? AND stage_kind='manual_extract'").bind(&unrelated).execute(&pool(&chain)).await.unwrap();
    let before: Vec<(String, String, i64, Option<String>)> = sqlx::query_as(
        "SELECT id,status,lease_epoch,lease_owner FROM job_stages WHERE job_id=? ORDER BY id",
    )
    .bind(&unrelated)
    .fetch_all(&pool(&chain))
    .await
    .unwrap();
    let (files, _) = authorize_files(&chain).await;
    let first = spawn(&files, None);
    wait_calls(&manual, MANUAL_AI_PATH, 1).await;
    let other = command(&files, false).output().unwrap();
    assert!(!other.status.success());
    assert!(String::from_utf8_lossy(&other.stderr).contains("instanceBusy"));
    let result = wait(first).await;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
    assert_eq!(manual.call_count("POST", MANUAL_AI_PATH), 1);
    assert_eq!(tripo.call_count("POST", TRIPO_SUBMIT_PATH), 1);
    assert_eq!(count(&chain, "jobs").await, 2);
    let after: Vec<(String, String, i64, Option<String>)> = sqlx::query_as(
        "SELECT id,status,lease_epoch,lease_owner FROM job_stages WHERE job_id=? ORDER BY id",
    )
    .bind(&unrelated)
    .fetch_all(&pool(&chain))
    .await
    .unwrap();
    assert_eq!(
        before, after,
        "claim and expired recovery must both stay within authorized job"
    );
    let journal = fs::read_dir(chain.app.dir().join("live-authorizations"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut j: Value = serde_json::from_slice(&fs::read(&journal).unwrap()).unwrap();
    j["jobId"] = Value::Null;
    private_json(&journal, &j);
    assert!(
        command(&files, false).output().unwrap().status.success(),
        "accepted job recovered through fixed business key even before journal jobId write"
    );
    assert_eq!(count(&chain, "jobs").await, 2);
    assert_eq!(count(&chain, "quotes").await, 2);
    cdn.shutdown();
    tripo.shutdown();
    manual.shutdown();
}

#[tokio::test]
#[ignore = "requires explicit job-failpoints xtask binary; localhost only"]
async fn pc04_cli_crash_lost_receipt_stays_unknown_but_known_receipt_resumes_without_repurchase() {
    for known in [false, true] {
        let manual = manual_ai_server(vec![respond_file(&responses_path("success.json"))]);
        let (cdn, url) = cdn_server("/model.glb", fixture_bytes("sample-model.glb"));
        let tripo = tripo_server(vec![submit_success()], vec![task_success_with(&url)]);
        let chain = chain("pc04-crash", &tripo, &manual).await;
        let (files, _) = authorize_files(&chain).await;
        let crashed = wait(spawn(
            &files,
            Some(if known {
                "paid_after_receipt_before_advance:exit"
            } else {
                "paid_after_response_before_receipt:exit"
            }),
        ))
        .await;
        assert_eq!(crashed.status.code(), Some(90));
        assert_eq!(tripo.call_count("POST", TRIPO_SUBMIT_PATH), 1);
        tokio::time::sleep(Duration::from_millis(3200)).await;
        let resumed = wait(spawn(&files, None)).await;
        let report = output_json(&resumed);
        assert_eq!(resumed.status.success(), known, "{report}");
        assert_eq!(
            report["status"],
            if known {
                "needs_review"
            } else {
                "submission_unknown"
            }
        );
        if !known {
            let cost = report["costs"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["provider"] == "tripo")
                .unwrap();
            assert!(matches!(
                cost["state"].as_str(),
                Some("reserved" | "unknown")
            ));
            assert!(cost["actual"].is_null());
            assert!(cost["reserved"].as_i64().unwrap() > 0);
            assert!(!report["savedAssets"].as_array().unwrap().is_empty());
        }
        let replay = command(&files, false).output().unwrap();
        assert_eq!(replay.status.success(), known);
        assert_eq!(output_json(&replay)["jobId"], report["jobId"]);
        assert_eq!(tripo.call_count("POST", TRIPO_SUBMIT_PATH), 1);
        assert_eq!(manual.call_count("POST", MANUAL_AI_PATH), 1);
        assert_eq!(count(&chain, "jobs").await, 1);
        assert_eq!(count(&chain, "quotes").await, 1);
        assert_eq!(count(&chain, "manual_releases").await, 0);
        cdn.shutdown();
        tripo.shutdown();
        manual.shutdown();
    }
}

#[tokio::test]
#[ignore = "requires explicit job-failpoints xtask binary; localhost only"]
async fn pc04_cli_safe_retry_requires_exact_scope_and_unknown_never_rebuys() {
    for (allowed, unknown) in [(false, false), (true, false), (true, true)] {
        let first = if unknown {
            Step::Disconnect
        } else {
            Step::Respond {
                response: ResponseSpec {
                    status: 429,
                    headers: BTreeMap::from([("retry-after".into(), "0".into())]),
                    body: BodySpec::Json {
                        json: json!({"error":{"message":"fixture rejected before processing"}}),
                    },
                },
            }
        };
        let manual = manual_ai_server(vec![first, respond_file(&responses_path("success.json"))]);
        let (cdn, url) = cdn_server("/model.glb", fixture_bytes("sample-model.glb"));
        let tripo = tripo_server(vec![submit_success()], vec![task_success_with(&url)]);
        let chain = chain("pc04-retry", &tripo, &manual).await;
        let (files, plan) = authorize_files(&chain).await;
        if allowed {
            let stage = plan["stages"]
                .as_array()
                .unwrap()
                .iter()
                .find(|s| s["stageKind"] == "manual_extract")
                .unwrap();
            let mut budget = budget_for(&plan);
            budget["retryScopes"] = json!([{"stageKind":stage["stageKind"],"batchIndex":stage["batchIndex"],"stageInputHash":stage["stageInputHash"],"operation":"safeRetry","maxAdditionalAttempts":1}]);
            private_json(&files.budget, &budget);
        }
        let out = wait(spawn(&files, None)).await;
        let report = output_json(&out);
        assert_eq!(out.status.success(), allowed && !unknown, "{report}");
        assert_eq!(
            manual.call_count("POST", MANUAL_AI_PATH),
            if allowed && !unknown { 2 } else { 1 }
        );
        if unknown {
            assert_eq!(report["status"], "submission_unknown");
            let cost = report["costs"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["provider"] == "manual_ai")
                .unwrap();
            assert_eq!(cost["state"], "unknown");
            assert!(cost["actual"].is_null());
        }
        let calls = manual.request_total() + tripo.request_total() + cdn.request_total();
        let _ = command(&files, false).output().unwrap();
        assert_eq!(
            calls,
            manual.request_total() + tripo.request_total() + cdn.request_total()
        );
        assert_eq!(count(&chain, "jobs").await, 1);
        cdn.shutdown();
        tripo.shutdown();
        manual.shutdown();
    }
}

#[tokio::test]
#[ignore = "requires explicit job-failpoints xtask binary; localhost only"]
async fn pc04_cli_runtime_expiry_configuration_scope_and_actual_risk_stop_new_calls() {
    for change in ["expiry", "configuration", "stage", "actual"] {
        let manual = manual_ai_server(vec![delay(
            respond_file(&responses_path("success.json")),
            500,
        )]);
        let tripo = tripo_server(vec![], vec![]);
        let chain = chain("pc04-runtime", &tripo, &manual).await;
        let (files, plan) = authorize_files(&chain).await;
        if change == "expiry" {
            let mut b = budget_for(&plan);
            b["expiresAt"] = json!(Timestamp::now().checked_add_millis(350).unwrap());
            private_json(&files.budget, &b);
        }
        let child = spawn(&files, None);
        wait_calls(&manual, MANUAL_AI_PATH, 1).await;
        match change {
            "configuration" => {
                let path = chain.app.dir().join("price-catalog.toml");
                let text = fs::read_to_string(&path).unwrap();
                fs::write(path, text.replace("credits = \"30\"", "credits = \"31\"")).unwrap();
            }
            "stage" => {
                sqlx::query("UPDATE job_stages SET input_hash=? WHERE stage_kind='tripo_submit'")
                    .bind("f".repeat(64))
                    .execute(&pool(&chain))
                    .await
                    .unwrap();
            }
            "actual" => {
                sqlx::query("UPDATE cost_ledger SET actual=? WHERE provider='tripo'")
                    .bind(plan["requiredLimits"]["creditMinor"].as_i64().unwrap() + 1)
                    .execute(&pool(&chain))
                    .await
                    .unwrap();
            }
            _ => {}
        }
        let out = wait(child).await;
        let report = output_json(&out);
        assert!(!out.status.success(), "{change}: {report}");
        assert_eq!(manual.call_count("POST", MANUAL_AI_PATH), 1);
        assert_eq!(
            tripo.request_total(),
            0,
            "no new upload, submit or query after gate change"
        );
        assert!(
            !report["savedAssets"].as_array().unwrap().is_empty(),
            "in-flight paid result preserved"
        );
        assert_eq!(count(&chain, "manual_releases").await, 0);
        assert_eq!(
            report["stoppedBy"],
            match change {
                "expiry" => "authorizationExpired",
                "configuration" => "planChanged",
                "stage" => "stageScopeChanged",
                _ => "budgetRisk",
            }
        );
        if change == "actual" {
            assert_eq!(report["budgetRisk"], true);
        }
        tripo.shutdown();
        manual.shutdown();
    }
}
