// CF-70/CF-71/CF-72/CF-73: security hardening for the dashboard login, session storage,
// and the wallet preflight.

use super::*;
use sea_orm::ConnectionTrait;

#[tokio::test]
async fn healthz_answers_plain_text_not_the_spa_fallback() {
    let ctx = setup().await;
    let req = Request::builder()
        .method("GET")
        .uri("/healthz")
        .body(Body::empty())
        .unwrap();
    let resp = ctx.router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&body);
    assert_eq!(text.trim(), "ok");
    // The SPA fallback serves HTML with 200 for any unknown path; this must not be that.
    assert!(
        !text.contains("<html"),
        "healthz must not be served by the SPA fallback: {text}"
    );
}

#[tokio::test]
async fn an_unknown_path_still_reaches_the_spa_fallback() {
    // Guards the route registration: /healthz must be a real route, not a side effect of
    // the fallback answering everything.
    let ctx = setup().await;
    let req = Request::builder()
        .method("GET")
        .uri("/definitely-not-a-route-cf70")
        .body(Body::empty())
        .unwrap();
    let resp = ctx.router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&body);
    assert_ne!(
        text.trim(),
        "ok",
        "the fallback must not answer like the health route"
    );
}

#[tokio::test]
async fn readyz_reports_database_reachability() {
    let ctx = setup().await;
    let req = Request::builder()
        .method("GET")
        .uri("/readyz")
        .body(Body::empty())
        .unwrap();
    let resp = ctx.router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let value: Value = serde_json::from_slice(&body).expect("readyz returns JSON");
    assert_eq!(value["status"], json!("ready"));
    assert_eq!(value["database_reachable"], json!(true));
    assert!(value["database_backend"].is_string());
}

#[tokio::test]
async fn dashboard_login_is_throttled_after_repeated_failures() {
    let ctx = setup().await;
    // CAPTCHA would reject the attempt before the password is verified, which is the
    // wrong failure for this test. Disable it with an admin session, exactly as an
    // operator would.
    ctx.state
        .user_store
        .create_user(
            "cf70-admin",
            "admin-password-12",
            monoize::users::UserRole::Admin,
            None,
        )
        .await
        .expect("admin creates");
    let cookie = dashboard_session_cookie(&ctx, "cf70-admin", "admin-password-12").await;
    let update = ctx
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/dashboard/settings")
                .header(CONTENT_TYPE, "application/json")
                .header("cookie", &cookie)
                .body(Body::from(json!({"captcha_enabled": false}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(update.status(), StatusCode::OK, "captcha must be disabled for this test");

    let attempt = |username: &str| {
        let router = ctx.router.clone();
        let body = json!({
            "username": username,
            "password": "definitely-the-wrong-password",
            "captcha_token": "unused"
        });
        async move {
            let req = Request::builder()
                .method("POST")
                .uri("/api/dashboard/auth/login")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap();
            router.oneshot(req).await.unwrap().status()
        }
    };

    // The first failures are answered as invalid credentials.
    for _ in 0..5 {
        let status = attempt("throttle-probe").await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "a failed login before the limit is 401"
        );
    }
    // The next attempt is refused without reaching password verification.
    let status = attempt("throttle-probe").await;
    assert_eq!(
        status,
        StatusCode::TOO_MANY_REQUESTS,
        "the sixth failure for one account must be throttled"
    );

    // A different username is unaffected.
    let status = attempt("throttle-probe-other").await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "throttling one username must not throttle another"
    );
}

#[tokio::test]
async fn a_session_token_is_not_stored_in_plaintext() {
    let ctx = setup().await;
    let user = ctx
        .state
        .user_store
        .create_user("cf71-owner", "password123", monoize::users::UserRole::User, None)
        .await
        .expect("user creates");
    let session = ctx
        .state
        .user_store
        .create_session(&user.id, 7)
        .await
        .expect("session creates");

    // The caller still receives a usable token...
    assert!(session.token.starts_with("urp_session_"));
    assert!(
        ctx.state
            .user_store
            .get_session_by_token(&session.token)
            .await
            .expect("lookup succeeds")
            .is_some(),
        "the plaintext token must still authenticate"
    );

    // ...but the row must not hold it verbatim.
    let write = ctx.state.db_pool.write().await;
    let row = write
        .query_one(ctx.state.db_pool.stmt(
            "SELECT token FROM sessions WHERE id = $1",
            vec![session.id.clone().into()],
        ))
        .await
        .expect("row reads")
        .expect("row exists");
    let stored: String = row.try_get("", "token").expect("token decodes");
    assert_ne!(
        stored, session.token,
        "the session token must not be stored verbatim"
    );
    assert_eq!(
        stored,
        monoize::users::UserStore::hash_session_token_public(&session.token),
        "the stored value must be the token digest"
    );
}

#[tokio::test]
async fn in_flight_spend_is_counted_and_released_with_the_funding_scope() {
    // CF-73: while a request holds its funding scope, its cost ceiling is subtracted from
    // the spendable balance so a concurrent request cannot spend the same money. The
    // guard releases on drop, so a finished request stops counting.
    let state = &setup().await.state;

    assert_eq!(state.in_flight_spend.reserved_for("cf73-user"), 0);
    {
        let _first = state.in_flight_spend.reserve("cf73-user", 1_000);
        assert_eq!(
            state.in_flight_spend.reserved_for("cf73-user"),
            1_000,
            "a held reservation must be visible to the preflight"
        );
        // Cloning the scope must not double-count or release early.
        let clone = _first.clone();
        // Bind the second guard: an unbound value would drop at the end of this statement
        // and release immediately, which is not what this test is asserting.
        let _second = state.in_flight_spend.reserve("cf73-user", 500);
        assert_eq!(state.in_flight_spend.reserved_for("cf73-user"), 1_500);
        drop(clone);
        assert_eq!(
            state.in_flight_spend.reserved_for("cf73-user"),
            1_500,
            "a clone shares one reservation, so dropping it must release nothing while              the original still holds it"
        );
    }
    assert_eq!(
        state.in_flight_spend.reserved_for("cf73-user"),
        0,
        "dropping the last guard must release the reservation"
    );
}
