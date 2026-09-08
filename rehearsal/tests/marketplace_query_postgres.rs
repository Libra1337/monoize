use monoize_lynshen_rehearsal::marketplace::{
    MarketplaceQuery, OfferQueryInput, QueryInput, create_postgres_query_fixture,
    create_sqlite_query_fixture,
};
use sqlx::{Connection, PgConnection, SqliteConnection};

async fn databases() -> Option<(SqliteConnection, PgConnection)> {
    let url = std::env::var("LYNSHEN_REHEARSAL_POSTGRES_URL").ok()?;
    let mut sqlite = SqliteConnection::connect("sqlite::memory:").await.unwrap();
    create_sqlite_query_fixture(&mut sqlite).await.unwrap();
    let mut postgres = PgConnection::connect(&url).await.unwrap();
    create_postgres_query_fixture(&mut postgres).await.unwrap();
    Some((sqlite, postgres))
}

#[tokio::test]
async fn postgres_and_sqlite_list_pages_are_identical() {
    let Some((mut sqlite, mut postgres)) = databases().await else {
        return;
    };
    for query in [None, Some("gPt".to_owned()), Some("模型".to_owned())] {
        let input = QueryInput {
            query,
            group: None,
            after: None,
            limit: 2,
        };
        let sqlite_page = MarketplaceQuery::list_sqlite(&mut sqlite, input.clone())
            .await
            .unwrap();
        let postgres_page = MarketplaceQuery::list_postgres(&mut postgres, input)
            .await
            .unwrap();
        assert_eq!(postgres_page, sqlite_page);
    }
}

#[tokio::test]
async fn postgres_and_sqlite_offer_order_is_identical() {
    let Some((mut sqlite, mut postgres)) = databases().await else {
        return;
    };
    let input = OfferQueryInput {
        group: "Alpha".to_owned(),
        model: "GPT-4o".to_owned(),
        after: None,
        limit: 1,
    };
    let sqlite_first = MarketplaceQuery::offers_sqlite(&mut sqlite, input.clone())
        .await
        .unwrap();
    let postgres_first = MarketplaceQuery::offers_postgres(&mut postgres, input)
        .await
        .unwrap();
    assert_eq!(postgres_first, sqlite_first);

    let input = OfferQueryInput {
        group: "Alpha".to_owned(),
        model: "GPT-4o".to_owned(),
        after: sqlite_first.next_key,
        limit: 1,
    };
    assert_eq!(
        MarketplaceQuery::offers_postgres(&mut postgres, input.clone())
            .await
            .unwrap(),
        MarketplaceQuery::offers_sqlite(&mut sqlite, input)
            .await
            .unwrap()
    );
}
