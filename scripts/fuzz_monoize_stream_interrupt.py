#!/usr/bin/env python3
"""Abort Monoize streams at item boundaries and replay the visible history."""

from __future__ import annotations

import copy
import hashlib
import json
import os
import time
import urllib.error
import urllib.request
from typing import Any


URL = os.environ.get("MONOIZE_PROBE_URL", "http://127.0.0.1:40550/v1/responses")
API_KEY = os.environ["MONOIZE_PROBE_API_KEY"]


def base_payload() -> dict:
    return {
        "model": "gpt-5.6-sol",
        "input": [
            {
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": (
                            "Carefully determine whether 'tacocat' is a palindrome. "
                            "Then call echo exactly once with a short conclusion. "
                            "Do not answer directly."
                        ),
                    }
                ],
            }
        ],
        "tools": [
            {
                "type": "function",
                "name": "echo",
                "description": "Return the supplied text.",
                "parameters": {
                    "type": "object",
                    "properties": {"text": {"type": "string"}},
                    "required": ["text"],
                    "additionalProperties": False,
                },
                "strict": True,
            }
        ],
        "tool_choice": "auto",
        "reasoning": {"effort": "high", "summary": "auto"},
        "include": ["reasoning.encrypted_content"],
        "max_output_tokens": 4096,
        "stream": True,
        "store": False,
    }


def request(payload: dict):
    return urllib.request.urlopen(
        urllib.request.Request(
            URL,
            data=json.dumps(payload, separators=(",", ":")).encode(),
            method="POST",
            headers={
                "Authorization": f"Bearer {API_KEY}",
                "Content-Type": "application/json",
                "User-Agent": "reqwest",
            },
        ),
        timeout=180,
    )


def parse_stream_events(body: bytes) -> list[dict[str, Any]]:
    events: list[dict[str, Any]] = []
    event_name = None
    data_lines: list[str] = []
    for raw_line in body.splitlines():
        line = raw_line.decode(errors="replace").rstrip("\r\n")
        if line.startswith("event:"):
            event_name = line[6:].strip()
        elif line.startswith("data:"):
            data_lines.append(line[5:].strip())
        elif not line and data_lines:
            raw = "\n".join(data_lines)
            data_lines = []
            if raw == "[DONE]":
                continue
            try:
                data = json.loads(raw)
            except json.JSONDecodeError:
                continue
            if event_name and "type" not in data:
                data["type"] = event_name
            events.append(data)
            event_name = None
    return events


def summarize_failure(body: bytes) -> dict[str, Any]:
    events = parse_stream_events(body)
    failure = next(
        (event for event in events if event.get("type") == "response.failed"),
        None,
    )
    if not isinstance(failure, dict):
        return {"event_types": [event.get("type") for event in events[-6:]]}
    response = failure.get("response")
    if not isinstance(response, dict):
        return {"event_types": [event.get("type") for event in events[-6:]]}
    error = response.get("error")
    if not isinstance(error, dict):
        return {"event_types": [event.get("type") for event in events[-6:]]}
    return {
        "response_id": response.get("id"),
        "error_code": error.get("code"),
        "error_message": error.get("message"),
        "event_types": [event.get("type") for event in events[-6:]],
    }


def fingerprint(item: dict | None) -> dict | None:
    if not item:
        return None
    encrypted = item.get("encrypted_content")
    return {
        "type": item.get("type"),
        "id": item.get("id"),
        "status": item.get("status"),
        "encrypted_chars": len(encrypted) if isinstance(encrypted, str) else None,
        "encrypted_sha256": (
            hashlib.sha256(encrypted.encode()).hexdigest()
            if isinstance(encrypted, str)
            else None
        ),
        "call_id": item.get("call_id"),
        "arguments_chars": (
            len(item.get("arguments")) if isinstance(item.get("arguments"), str) else None
        ),
    }


def abort_at(cutoff: str) -> dict[str, dict | None]:
    state: dict[str, dict | None] = {
        "reasoning_added": None,
        "reasoning_done": None,
        "call_added": None,
        "call_done": None,
    }
    response = request(base_payload())
    event_name = None
    data_lines: list[str] = []
    try:
        while True:
            raw_line = response.readline()
            if not raw_line:
                break
            line = raw_line.decode(errors="replace").rstrip("\r\n")
            if line.startswith("event:"):
                event_name = line[6:].strip()
            elif line.startswith("data:"):
                data_lines.append(line[5:].strip())
            elif not line and data_lines:
                raw = "\n".join(data_lines)
                data_lines = []
                if raw == "[DONE]":
                    continue
                try:
                    data = json.loads(raw)
                except json.JSONDecodeError:
                    continue
                kind = event_name or data.get("type")
                event_name = None
                item = data.get("item")
                observed = None
                if kind == "response.output_item.added" and isinstance(item, dict):
                    if item.get("type") == "reasoning":
                        state["reasoning_added"] = copy.deepcopy(item)
                        observed = "reasoning_added"
                    elif item.get("type") == "function_call":
                        state["call_added"] = copy.deepcopy(item)
                        observed = "call_added"
                elif kind == "response.output_item.done" and isinstance(item, dict):
                    if item.get("type") == "reasoning":
                        state["reasoning_done"] = copy.deepcopy(item)
                        observed = "reasoning_done"
                    elif item.get("type") == "function_call":
                        state["call_done"] = copy.deepcopy(item)
                        observed = "call_done"
                elif kind == "response.function_call_arguments.delta":
                    call = state.get("call_added")
                    if isinstance(call, dict):
                        call["arguments"] = str(call.get("arguments") or "") + str(
                            data.get("delta") or ""
                        )
                    observed = "call_arguments_delta"
                if observed == cutoff:
                    break
    finally:
        response.close()
    print(
        json.dumps(
            {
                "cutoff": cutoff,
                "visible": {key: fingerprint(value) for key, value in state.items()},
            },
            separators=(",", ":"),
        ),
        flush=True,
    )
    return state


def clean_reasoning(item: dict) -> dict:
    value = copy.deepcopy(item)
    value.pop("id", None)
    value.pop("status", None)
    value.pop("started_at", None)
    value.pop("duration", None)
    return value


def clean_call(item: dict) -> dict:
    value = copy.deepcopy(item)
    value.pop("id", None)
    value.pop("status", None)
    return value


def replay_input(state: dict[str, dict | None], cutoff: str) -> list[dict]:
    if cutoff == "reasoning_added":
        reasoning = state["reasoning_added"]
        assert isinstance(reasoning, dict)
        return [
            clean_reasoning(reasoning),
            {"role": "assistant", "content": "Interrupted."},
            {"role": "user", "content": "Reply with OK."},
        ]
    if cutoff == "reasoning_done":
        reasoning = state["reasoning_done"]
        assert isinstance(reasoning, dict)
        return [
            clean_reasoning(reasoning),
            {"role": "assistant", "content": "Interrupted."},
            {"role": "user", "content": "Reply with OK."},
        ]
    reasoning = state["reasoning_done"]
    call = state["call_done"] or state["call_added"]
    assert isinstance(reasoning, dict) and isinstance(call, dict)
    call = clean_call(call)
    return [
        clean_reasoning(reasoning),
        call,
        {
            "type": "function_call_output",
            "call_id": call["call_id"],
            "output": (
                "Skipped due to queued user message. Do not count this skipped result as "
                "completed work or verification."
            ),
        },
        {"role": "user", "content": "Reply with OK."},
    ]


def run_replay(cutoff: str, state: dict[str, dict | None]) -> None:
    payload = base_payload()
    payload["input"] = replay_input(state, cutoff)
    started = time.monotonic()
    try:
        with request(payload) as response:
            status = response.status
            body = response.read()
    except urllib.error.HTTPError as error:
        status = error.code
        body = error.read()
    text = body.decode(errors="replace").lower()
    outcome = (
        "failed"
        if status >= 500 or "response.failed" in text
        else "rejected"
        if status >= 400
        else "accepted"
        if "response.completed" in text or "response.incomplete" in text
        else "unknown"
    )
    print(
        json.dumps(
            {
                "replay_cutoff": cutoff,
                "http_status": status,
                "outcome": outcome,
                "elapsed_seconds": round(time.monotonic() - started, 3),
                "response_bytes": len(body),
                "failure": summarize_failure(body) if outcome == "failed" else None,
            },
            separators=(",", ":"),
        ),
        flush=True,
    )


def main() -> int:
    selected = os.environ.get(
        "MONOIZE_CUTOFFS",
        "reasoning_added,reasoning_done,call_added,call_arguments_delta,call_done",
    )
    for cutoff in tuple(part.strip() for part in selected.split(",") if part.strip()):
        state = abort_at(cutoff)
        run_replay(cutoff, state)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
