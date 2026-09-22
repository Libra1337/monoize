//! Startup seeding: settings defaults, builtin templates, node packs.

use crate::db::Pool;
use serde_json::{json, Value};

async fn seed_setting(db: &Pool, key: &str, value: &str) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO settings (key, value, updated_at) VALUES ($1, $2, $3) \
         ON CONFLICT (key) DO NOTHING",
    )
    .bind(key)
    .bind(value)
    .bind(crate::now_rfc3339())
    .execute(db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

struct TemplateSeed {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    sort: i64,
    graph: Value,
}

fn node(id: &str, kind: &str, x: f64, y: f64, params: Value) -> Value {
    json!({ "id": id, "kind": kind, "x": x, "y": y, "params": params })
}

fn edge(id: &str, source: &str, source_port: &str, target: &str, target_port: &str) -> Value {
    json!({
        "id": id,
        "source": source,
        "sourcePort": source_port,
        "target": target,
        "targetPort": target_port,
    })
}

fn templates() -> Vec<TemplateSeed> {
    vec![
        TemplateSeed {
            id: "oneclick-standard",
            name: "One-click short film",
            description: "Topic in, finished film out: script, storyboard, shots, voice-over, subtitles, assembly.",
            sort: 1,
            graph: json!({
                "nodes": [
                    node("script", "script", 60.0, 300.0, json!({
                        "system": "You are a professional short-video script writer. Write a concise, vivid narration script for the given topic. Output only the script text.",
                        "prompt": ""
                    })),
                    node("board", "storyboard", 340.0, 300.0, json!({ "count": 6 })),
                    node("material", "material", 620.0, 160.0, json!({})),
                    node("voice", "tts", 620.0, 460.0, json!({ "voice": "alloy", "speed": 1.0 })),
                    node("subs", "subtitle", 900.0, 460.0, json!({})),
                    node("final", "assemble", 1180.0, 300.0, json!({
                        "resolution": "1280x720", "fps": 30
                    })),
                ],
                "edges": [
                    edge("e1", "script", "text", "board", "text"),
                    edge("e2", "board", "shots", "material", "text"),
                    edge("e3", "script", "text", "voice", "text"),
                    edge("e4", "script", "text", "subs", "text"),
                    edge("e5", "material", "video", "final", "clips"),
                    edge("e6", "voice", "audio", "final", "audio"),
                    edge("e7", "subs", "subtitle", "final", "subtitle"),
                ],
            }),
        },
        TemplateSeed {
            id: "blank-canvas",
            name: "Blank canvas",
            description: "An empty canvas with one script node.",
            sort: 2,
            graph: json!({
                "nodes": [
                    node("script", "script", 120.0, 200.0, json!({
                        "system": "You are a professional video script writer.",
                        "prompt": ""
                    })),
                ],
                "edges": [],
            }),
        },
        TemplateSeed {
            id: "ai-film",
            name: "AI-animated film",
            description: "Storyboard then per-shot text-to-video generation with an optional first-frame image chain.",
            sort: 3,
            graph: json!({
                "nodes": [
                    node("script", "script", 60.0, 240.0, json!({
                        "system": "You are a professional short-film script writer.",
                        "prompt": ""
                    })),
                    node("board", "storyboard", 340.0, 240.0, json!({ "count": 6 })),
                    node("frames", "image", 620.0, 120.0, json!({
                        "size": "1280x720", "style": "cinematic, high detail"
                    })),
                    node("shots", "video", 900.0, 240.0, json!({
                        "seconds": "5", "size": "1280x720"
                    })),
                    node("voice", "tts", 620.0, 480.0, json!({ "voice": "alloy" })),
                    node("subs", "subtitle", 900.0, 480.0, json!({})),
                    node("final", "assemble", 1180.0, 300.0, json!({})),
                ],
                "edges": [
                    edge("e1", "script", "text", "board", "text"),
                    edge("e2", "board", "shots", "frames", "text"),
                    edge("e3", "board", "shots", "shots", "text"),
                    edge("e4", "frames", "image", "shots", "image"),
                    edge("e5", "script", "text", "voice", "text"),
                    edge("e6", "script", "text", "subs", "text"),
                    edge("e7", "shots", "video", "final", "clips"),
                    edge("e8", "voice", "audio", "final", "audio"),
                    edge("e9", "subs", "subtitle", "final", "subtitle"),
                ],
            }),
        },
        TemplateSeed {
            id: "storyboard-only",
            name: "Storyboard workshop",
            description: "Draft a script, split it into shots, and inspect the storyboard JSON before going further.",
            sort: 4,
            graph: json!({
                "nodes": [
                    node("script", "script", 80.0, 200.0, json!({
                        "system": "You are a professional video script writer.",
                        "prompt": ""
                    })),
                    node("board", "storyboard", 380.0, 200.0, json!({ "count": 8 })),
                ],
                "edges": [edge("e1", "script", "text", "board", "text")],
            }),
        },
    ]
}

fn node_packs() -> Vec<(&'static str, &'static str, &'static str, Value)> {
    vec![
        (
            "core",
            "Core",
            "Script, storyboard, and note primitives.",
            json!(["note", "script", "storyboard"]),
        ),
        (
            "visual",
            "Visual generation",
            "Image and video generation nodes.",
            json!(["image", "video"]),
        ),
        (
            "pipeline",
            "Delivery pipeline",
            "Stock material, voice-over, subtitles, and final assembly.",
            json!(["material", "tts", "subtitle", "assemble"]),
        ),
    ]
}

pub async fn run(db: &Pool) -> Result<(), String> {
    seed_setting(db, "price_image_nano", "4000000").await?;
    seed_setting(db, "price_video_nano", "500000000").await?;
    seed_setting(db, "price_tts_nano", "20000000").await?;
    seed_setting(db, "price_assemble_nano", "10000000").await?;
    seed_setting(db, "price_material_nano", "0").await?;
    seed_setting(db, "price_subtitle_nano", "0").await?;

    let now = crate::now_rfc3339();
    for template in templates() {
        sqlx::query(
            "INSERT INTO templates (id, source, name, description, graph_json, params_json, \
             enabled, sort, created_at, updated_at) \
             VALUES ($1, 'builtin', $2, $3, $4, '{}', 1, $5, $6, $6) \
             ON CONFLICT (id) DO UPDATE SET graph_json = excluded.graph_json, \
             name = excluded.name, description = excluded.description, \
             sort = excluded.sort, updated_at = excluded.updated_at",
        )
        .bind(template.id)
        .bind(template.name)
        .bind(template.description)
        .bind(template.graph.to_string())
        .bind(template.sort)
        .bind(&now)
        .execute(db)
        .await
        .map_err(|e| format!("seed template {}: {e}", template.id))?;
    }

    for (id, name, description, nodes) in node_packs() {
        sqlx::query(
            "INSERT INTO node_packs (id, name, version, description, nodes_json, enabled, \
             source, created_at, updated_at) \
             VALUES ($1, $2, '1.0.0', $3, $4, 1, 'builtin', $5, $5) \
             ON CONFLICT (id) DO UPDATE SET nodes_json = excluded.nodes_json, \
             name = excluded.name, description = excluded.description, \
             updated_at = excluded.updated_at",
        )
        .bind(id)
        .bind(name)
        .bind(description)
        .bind(nodes.to_string())
        .bind(&now)
        .execute(db)
        .await
        .map_err(|e| format!("seed node pack {id}: {e}"))?;
    }
    Ok(())
}

/// Build the one-click graph from the user request (AP-U4).
pub fn build_oneclick_graph(
    topic: &str,
    style: &str,
    voice: &str,
    shot_count: usize,
    material_mode: &str,
    subtitle: bool,
    resolution: &str,
) -> Value {
    let style_line = if style.is_empty() {
        String::new()
    } else {
        format!(" Overall style: {style}.")
    };
    let mut nodes = vec![
        node("script", "script", 60.0, 300.0, json!({
            "system": format!("You are a professional short-video script writer. Write a concise, vivid narration script for the given topic.{style_line}"),
            "prompt": topic,
        })),
        node("board", "storyboard", 340.0, 300.0, json!({ "count": shot_count })),
        node(
            if material_mode == "stock" { "material" } else { "frames" },
            if material_mode == "stock" { "material" } else { "image" },
            620.0,
            160.0,
            json!({
                "size": resolution,
                "style": if style.is_empty() { "cinematic, high detail" } else { style },
            }),
        ),
        node("voice", "tts", 620.0, 460.0, json!({ "voice": voice, "speed": 1.0 })),
        node("final", "assemble", 1180.0, 300.0, json!({
            "resolution": resolution, "fps": 30
        })),
    ];
    let mut edges = vec![
        edge("e1", "script", "text", "board", "text"),
        edge("e2", "board", "shots", if material_mode == "stock" { "material" } else { "frames" }, "text"),
        edge("e3", "script", "text", "voice", "text"),
        edge(
            "e5",
            if material_mode == "stock" { "material" } else { "frames" },
            if material_mode == "stock" { "video" } else { "image" },
            "final",
            "clips",
        ),
        edge("e6", "voice", "audio", "final", "audio"),
    ];
    if subtitle {
        nodes.push(node("subs", "subtitle", 900.0, 460.0, json!({})));
        edges.push(edge("e4", "script", "text", "subs", "text"));
        edges.push(edge("e7", "subs", "subtitle", "final", "subtitle"));
    }
    json!({ "nodes": nodes, "edges": edges })
}
