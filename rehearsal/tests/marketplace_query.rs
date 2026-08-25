use monoize_lynshen_rehearsal::marketplace::{
    MarketplaceQuery, QueryInput, create_sqlite_query_fixture,
};
use sqlx::{Connection, SqliteConnection};

async fn database() -> SqliteConnection {
    let mut db = SqliteConnection::connect("sqlite::memory:").await.unwrap();
    create_sqlite_query_fixture(&mut db).await.unwrap();
    db
}

#[tokio::test]
async fn list_keeps_groups_separate_and_uses_binary_order() {
    let mut db = database().await;
    let page = MarketplaceQuery::list_sqlite(
        &mut db,
        QueryInput {
            query: None,
            group: None,
            after: None,
            limit: 3,
        },
    )
    .await
    .unwrap();

    assert_eq!(
        page.items
            .iter()
            .map(|item| (item.group.as_str(), item.model.as_str()))
            .collect::<Vec<_>>(),
        vec![("Alpha", "GPT-4o"), ("Alpha", "模型-A"), ("Beta", "GPT-4o")]
    );
    assert_eq!(page.next_key, None);
}

#[tokio::test]
async fn search_is_ascii_case_insensitive_and_non_ascii_literal() {
    let mut db = database().await;
    let ascii = MarketplaceQuery::list_sqlite(
        &mut db,
        QueryInput {
            query: Some("gPt".to_owned()),
            group: None,
            after: None,
            limit: 50,
        },
    )
    .await
    .unwrap();
    assert_eq!(ascii.items.len(), 2);

    let non_ascii = MarketplaceQuery::list_sqlite(
        &mut db,
        QueryInput {
            query: Some("模型".to_owned()),
            group: None,
            after: None,
            limit: 50,
        },
    )
    .await
    .unwrap();
    assert_eq!(non_ascii.items.len(), 1);
    assert_eq!(non_ascii.items[0].model, "模型-A");
}

#[tokio::test]
async fn keyset_page_resumes_strictly_after_final_key() {
    let mut db = database().await;
    let first = MarketplaceQuery::list_sqlite(
        &mut db,
        QueryInput {
            query: None,
            group: None,
            after: None,
            limit: 2,
        },
    )
    .await
    .unwrap();
    let second = MarketplaceQuery::list_sqlite(
        &mut db,
        QueryInput {
            query: None,
            group: None,
            after: first.next_key,
            limit: 2,
        },
    )
    .await
    .unwrap();

    assert_eq!(first.items.len(), 2);
    assert_eq!(second.items.len(), 1);
    assert_eq!(second.items[0].group, "Beta");
}

#[tokio::test]
async fn provider_offers_are_counted_within_each_group_model() {
    let mut db = database().await;
    let page = MarketplaceQuery::list_sqlite(
        &mut db,
        QueryInput {
            query: Some("gpt-4o".to_owned()),
            group: Some("Alpha".to_owned()),
            after: None,
            limit: 50,
        },
    )
    .await
    .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].offer_count, 2);
}
