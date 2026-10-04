//! ADR-044：视图候选图与一次性视图排列。
//!
//! 守住的合同：
//! - 候选只接受本物品的照片资产；同一资产重复登记幂等；
//! - 删除是软删除（列表不再返回）、可撤销；
//! - `PUT /photos/arrangement` 一次性替换槽位：两个视图互换不撞唯一索引、同资产保留照片 id、
//!   清空槽位即删除该照片；同一资产放两个视图 / 未知视图 / 外部资产 → 4xx 且不改变现有排列；
//! - 不配置说明书 AI 时 classify 不失败（候选保存为未判断）。

mod common;

use std::path::Path;

use axum::http::{Method, StatusCode};
use common::TestApp;
use serde_json::{Value, json};

const PASSWORD: &str = "test-password-view-candidates-7c2e";

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/assets")
            .join(name),
    )
    .unwrap()
}

async fn logged_in() -> (TestApp, String, String) {
    let app = TestApp::new("view-candidates").await;
    app.set_admin_password(PASSWORD).await;
    let r = app
        .call(Method::POST, "/api/v1/auth/login")
        .json(&json!({ "password": PASSWORD }))
        .send()
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let csrf = r.json()["data"]["csrfToken"].as_str().unwrap().to_owned();
    (app, r.session_cookie(), csrf)
}

async fn create_item(app: &TestApp, cookie: &str, csrf: &str) -> String {
    let r = app
        .call(Method::POST, "/api/v1/items")
        .cookie(cookie)
        .csrf(csrf)
        .json(&json!({ "name": "相机", "model": "VC-1" }))
        .send()
        .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    r.json()["data"]["id"].as_str().unwrap().to_owned()
}

async fn upload(
    app: &TestApp,
    cookie: &str,
    csrf: &str,
    item: &str,
    purpose: &str,
    name: &str,
) -> String {
    let boundary = "----em-view-candidates";
    let mime = if name.ends_with(".png") {
        "image/png"
    } else {
        "image/jpeg"
    };
    let mut body = format!("--{boundary}\r\nContent-Disposition: form-data; name=\"purpose\"\r\n\r\n{purpose}\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: {mime}\r\n\r\n").into_bytes();
    body.extend(fixture(name));
    body.extend(format!("\r\n--{boundary}--\r\n").into_bytes());
    let r = app
        .call(Method::POST, &format!("/api/v1/items/{item}/assets"))
        .cookie(cookie)
        .csrf(csrf)
        .raw_body(
            Some(&format!("multipart/form-data; boundary={boundary}")),
            body,
        )
        .send()
        .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    r.json()["data"]["id"].as_str().unwrap().to_owned()
}

async fn arrange(
    app: &TestApp,
    cookie: &str,
    csrf: &str,
    item: &str,
    slots: Value,
) -> (StatusCode, Value) {
    let r = app
        .call(
            Method::PUT,
            &format!("/api/v1/items/{item}/photos/arrangement"),
        )
        .cookie(cookie)
        .csrf(csrf)
        .json(&json!({ "slots": slots }))
        .send()
        .await;
    let status = r.status;
    (
        status,
        if r.text().is_empty() {
            Value::Null
        } else {
            r.json()
        },
    )
}

async fn photos(app: &TestApp, cookie: &str, item: &str) -> Vec<(String, String, String)> {
    let r = app
        .call(Method::GET, &format!("/api/v1/items/{item}/photos"))
        .cookie(cookie)
        .send()
        .await;
    r.json()["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["view"].as_str().unwrap().to_owned(),
                p["assetId"].as_str().unwrap().to_owned(),
                p["id"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

async fn count_candidates(app: &TestApp, cookie: &str, item: &str) -> usize {
    app.call(
        Method::GET,
        &format!("/api/v1/items/{item}/view-candidates"),
    )
    .cookie(cookie)
    .send()
    .await
    .json()["data"]
        .as_array()
        .unwrap()
        .len()
}

#[tokio::test]
async fn candidates_are_idempotent_soft_deleted_and_restorable() {
    let (app, cookie, csrf) = logged_in().await;
    let item = create_item(&app, &cookie, &csrf).await;
    let asset = upload(
        &app,
        &cookie,
        &csrf,
        &item,
        "photo",
        "sample-photo-front.jpg",
    )
    .await;
    let create = |classify: bool| json!({ "assetId": asset, "pageNumber": 6, "source": "region", "classify": classify });
    let r = app
        .call(
            Method::POST,
            &format!("/api/v1/items/{item}/view-candidates"),
        )
        .cookie(&cookie)
        .csrf(&csrf)
        .json(&create(true))
        .send()
        .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    let candidate = r.json()["data"].clone();
    // 测试应用未配置说明书 AI：不失败，候选保存为未判断并注明原因。
    assert!(candidate["suggestedView"].is_null());
    assert!(
        candidate["note"].as_str().unwrap().contains("未配置"),
        "{candidate}"
    );
    let again = app
        .call(
            Method::POST,
            &format!("/api/v1/items/{item}/view-candidates"),
        )
        .cookie(&cookie)
        .csrf(&csrf)
        .json(&create(false))
        .send()
        .await;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(again.json()["data"]["id"], candidate["id"]);

    let id = candidate["id"].as_str().unwrap();
    assert_eq!(count_candidates(&app, &cookie, &item).await, 1);
    let d = app
        .call(
            Method::POST,
            &format!("/api/v1/items/{item}/view-candidates/{id}/dismiss"),
        )
        .cookie(&cookie)
        .csrf(&csrf)
        .send()
        .await;
    assert_eq!(d.status, StatusCode::NO_CONTENT);
    assert_eq!(count_candidates(&app, &cookie, &item).await, 0);
    let r = app
        .call(
            Method::POST,
            &format!("/api/v1/items/{item}/view-candidates/{id}/restore"),
        )
        .cookie(&cookie)
        .csrf(&csrf)
        .send()
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert_eq!(count_candidates(&app, &cookie, &item).await, 1);
}

#[tokio::test]
async fn candidates_reject_non_photo_assets_and_bad_source() {
    let (app, cookie, csrf) = logged_in().await;
    let item = create_item(&app, &cookie, &csrf).await;
    let page = upload(
        &app,
        &cookie,
        &csrf,
        &item,
        "pageImage",
        "sample-photo-front.jpg",
    )
    .await;
    let r = app
        .call(
            Method::POST,
            &format!("/api/v1/items/{item}/view-candidates"),
        )
        .cookie(&cookie)
        .csrf(&csrf)
        .json(&json!({ "assetId": page, "source": "region", "classify": false }))
        .send()
        .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.text());
    let photo = upload(
        &app,
        &cookie,
        &csrf,
        &item,
        "photo",
        "sample-photo-front.jpg",
    )
    .await;
    let r = app
        .call(
            Method::POST,
            &format!("/api/v1/items/{item}/view-candidates"),
        )
        .cookie(&cookie)
        .csrf(&csrf)
        .json(&json!({ "assetId": photo, "source": "web", "classify": false }))
        .send()
        .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn arrangement_swaps_clears_and_rejects_invalid_without_side_effects() {
    let (app, cookie, csrf) = logged_in().await;
    let item = create_item(&app, &cookie, &csrf).await;
    let a = upload(
        &app,
        &cookie,
        &csrf,
        &item,
        "photo",
        "sample-photo-front.jpg",
    )
    .await;
    let b = upload(
        &app,
        &cookie,
        &csrf,
        &item,
        "photo",
        "sample-photo-left.png",
    )
    .await;
    let c = upload(
        &app,
        &cookie,
        &csrf,
        &item,
        "photo",
        "sample-photo-front.jpg",
    )
    .await;

    let (status, _) = arrange(
        &app,
        &cookie,
        &csrf,
        &item,
        json!({ "front": a, "left": b, "detail": c }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let before = photos(&app, &cookie, &item).await;
    assert_eq!(
        before
            .iter()
            .map(|p| (p.0.as_str(), p.1.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("front", a.as_str()),
            ("left", b.as_str()),
            ("detail", c.as_str())
        ]
    );

    // 互换 front/left（逐条 PATCH 会撞唯一索引），并清空 detail。
    let (status, body) = arrange(
        &app,
        &cookie,
        &csrf,
        &item,
        json!({ "front": b, "left": a, "detail": null }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let after = photos(&app, &cookie, &item).await;
    assert_eq!(
        after
            .iter()
            .map(|p| (p.0.as_str(), p.1.as_str()))
            .collect::<Vec<_>>(),
        vec![("front", b.as_str()), ("left", a.as_str())]
    );
    let id_of = |list: &[(String, String, String)], asset: &str| {
        list.iter().find(|p| p.1 == asset).unwrap().2.clone()
    };
    assert_eq!(
        id_of(&before, &a),
        id_of(&after, &a),
        "同一资产保留原照片 id"
    );

    // 非法输入：不改变现有排列。
    for slots in [
        json!({ "front": a, "back": a }),
        json!({ "top": a }),
        json!({ "front": "not-an-asset" }),
    ] {
        let (status, _) = arrange(&app, &cookie, &csrf, &item, slots).await;
        assert!(status.is_client_error(), "{status}");
        assert_eq!(photos(&app, &cookie, &item).await, after);
    }
    let other = create_item(&app, &cookie, &csrf).await;
    let foreign = upload(
        &app,
        &cookie,
        &csrf,
        &other,
        "photo",
        "sample-photo-front.jpg",
    )
    .await;
    let (status, _) = arrange(&app, &cookie, &csrf, &item, json!({ "front": foreign })).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(photos(&app, &cookie, &item).await, after);
}
