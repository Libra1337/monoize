#!/usr/bin/env python3
"""Generate native Responses reasoning/tool pairs and fuzz their replay contract."""

from __future__ import annotations

import copy
import hashlib
import json
import os
import re
import sys
import time
import urllib.error
import urllib.request
import uuid
from dataclasses import dataclass


REQUEST_ID_PATTERN = re.compile(r"[0-9a-f]{8}-[0-9a-f-]{27}", re.IGNORECASE)


@dataclass
class HttpResult:
    status: int
    body: bytes
    request_id: str | None
    elapsed_seconds: float


def post_json(url: str, api_key: str, payload: dict) -> HttpResult:
    encoded = json.dumps(payload, separators=(",", ":")).encode()
    request = urllib.request.Request(
        url,
        data=encoded,
        method="POST",
        headers={
            "Authorization": f"Bearer {api_key}",
            "Content-Type": "application/json",
            "User-Agent": "reqwest",
        },
    )
    started = time.monotonic()
    try:
        with urllib.request.urlopen(request, timeout=180) as response:
            status = response.status
            body = response.read()
            request_id = response.headers.get("x-request-id")
    except urllib.error.HTTPError as error:
        status = error.code
        body = error.read()
        request_id = error.headers.get("x-request-id")
    if not request_id:
        match = REQUEST_ID_PATTERN.search(body.decode(errors="replace"))
        request_id = match.group(0) if match else None
    return HttpResult(status, body, request_id, time.monotonic() - started)


def parse_sse(body: bytes) -> list[tuple[str | None, dict]]:
    events: list[tuple[str | None, dict]] = []
    event_name: str | None = None
    data_lines: list[str] = []
    for line in body.decode(errors="replace").splitlines() + [""]:
        if line.startswith("event:"):
            event_name = line[6:].strip()
        elif line.startswith("data:"):
            data_lines.append(line[5:].strip())
        elif not line and data_lines:
            raw = "\n".join(data_lines)
            if raw != "[DONE]":
                try:
                    events.append((event_name, json.loads(raw)))
                except json.JSONDecodeError:
                    pass
            event_name = None
            data_lines = []
    return events


def event_type(name: str | None, data: dict) -> str:
    return name or str(data.get("type") or "")


def item_fingerprint(item: dict | None) -> dict | None:
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
        "name": item.get("name"),
    }


def extract_pair(result: HttpResult, label: str) -> tuple[dict, dict, dict]:
    if result.status != 200:
        raise RuntimeError(f"{label} generation returned HTTP {result.status}")
    events = parse_sse(result.body)
    added_reasoning = None
    done_reasoning = None
    terminal_response = None
    for name, data in events:
        kind = event_type(name, data)
        item = data.get("item")
        if kind == "response.output_item.added" and isinstance(item, dict):
            if item.get("type") == "reasoning":
                added_reasoning = item
        elif kind == "response.output_item.done" and isinstance(item, dict):
            if item.get("type") == "reasoning":
                done_reasoning = item
        elif kind == "response.completed":
            terminal_response = data.get("response")
    if not isinstance(terminal_response, dict):
        raise RuntimeError(f"{label} generation has no response.completed")
    output = terminal_response.get("output")
    if not isinstance(output, list):
        raise RuntimeError(f"{label} generation has no terminal output array")
    terminal_reasoning = next(
        (item for item in output if isinstance(item, dict) and item.get("type") == "reasoning"),
        None,
    )
    function_call = next(
        (
            item
            for item in output
            if isinstance(item, dict) and item.get("type") == "function_call"
        ),
        None,
    )
    if not isinstance(terminal_reasoning, dict) or not isinstance(function_call, dict):
        print(
            json.dumps(
                {
                    "generation_error": {
                        "label": label,
                        "response_status": terminal_response.get("status"),
                        "incomplete_details": terminal_response.get("incomplete_details"),
                        "output_types": [
                            item.get("type") for item in output if isinstance(item, dict)
                        ],
                    }
                },
                separators=(",", ":"),
            ),
            flush=True,
        )
        raise RuntimeError(f"{label} generation did not produce reasoning + function_call")
    encrypted = terminal_reasoning.get("encrypted_content")
    if not isinstance(encrypted, str) or not encrypted:
        raise RuntimeError(f"{label} terminal reasoning has no encrypted_content")
    metadata = {
        "label": label,
        "http_status": result.status,
        "request_id": result.request_id,
        "elapsed_seconds": round(result.elapsed_seconds, 3),
        "added": item_fingerprint(added_reasoning),
        "done": item_fingerprint(done_reasoning),
        "completed": item_fingerprint(terminal_reasoning),
        "function_call": item_fingerprint(function_call),
    }
    print(json.dumps({"generated": metadata}, separators=(",", ":")), flush=True)
    return terminal_reasoning, function_call, {
        "added_reasoning": copy.deepcopy(added_reasoning),
        "done_reasoning": copy.deepcopy(done_reasoning),
    }


def generation_payload(model: str) -> dict:
    return {
        "model": model,
        "input": [
            {
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": (
                            "Carefully determine whether the string 'tacocat' is a palindrome. "
                            "Then call the echo tool exactly once with a short conclusion. "
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


def replay_payload(model: str, reasoning: dict, call: dict, output: str) -> dict:
    return {
        "model": model,
        "input": [
            reasoning,
            call,
            {
                "type": "function_call_output",
                "call_id": call["call_id"],
                "output": output,
            },
            {
                "role": "user",
                "content": [{"type": "input_text", "text": "Reply with OK."}],
            },
        ],
        "tools": generation_payload(model)["tools"],
        "reasoning": {"effort": "low", "summary": "auto"},
        "include": ["reasoning.encrypted_content"],
        "max_output_tokens": 128,
        "stream": True,
        "store": False,
    }


def classify(result: HttpResult) -> str:
    text = result.body.decode(errors="replace").lower()
    if result.status >= 500 or "response.failed" in text:
        return "failed"
    if result.status >= 400:
        return "rejected"
    if "response.completed" in text or "response.incomplete" in text:
        return "accepted"
    return "unknown"


def run_case(url: str, api_key: str, name: str, payload: dict) -> None:
    result = post_json(url, api_key, payload)
    print(
        json.dumps(
            {
                "case": name,
                "http_status": result.status,
                "outcome": classify(result),
                "request_id": result.request_id,
                "elapsed_seconds": round(result.elapsed_seconds, 3),
                "response_bytes": len(result.body),
            },
            separators=(",", ":"),
        ),
        flush=True,
    )


def without_status(item: dict) -> dict:
    value = copy.deepcopy(item)
    value.pop("status", None)
    return value


def main() -> int:
    url = os.environ["CPA_RESPONSES_URL"]
    api_key = os.environ["MONOIZE_PROBE_API_KEY"]
    model = os.environ.get("CPA_PROBE_MODEL", "gpt-5.6-sol")

    actual_capture = None
    if os.environ.get("CPA_FUZZ_ACTUAL") == "1":
        actual_capture = json.load(sys.stdin)

    pair1 = extract_pair(post_json(url, api_key, generation_payload(model)), "pair1")
    pair2 = extract_pair(post_json(url, api_key, generation_payload(model)), "pair2")
    reasoning1, call1, stream1 = pair1
    reasoning2, call2, _ = pair2
    normal_output = json.dumps({"text": "fuzz-probe"}, separators=(",", ":"))
    skipped_output = (
        "Skipped due to queued user message. Do not count this skipped result as completed "
        "work or verification."
    )

    cases: list[tuple[str, dict]] = []
    cases.append(("valid_terminal", replay_payload(model, reasoning1, call1, normal_output)))
    cases.append(("skipped_tool_output", replay_payload(model, reasoning1, call1, skipped_output)))

    payload = replay_payload(model, without_status(reasoning1), call1, normal_output)
    cases.append(("strip_reasoning_status", payload))
    payload = replay_payload(model, reasoning1, without_status(call1), normal_output)
    cases.append(("strip_call_status", payload))
    payload = replay_payload(
        model, without_status(reasoning1), without_status(call1), normal_output
    )
    cases.append(("strip_both_status", payload))

    wrong_id = copy.deepcopy(reasoning1)
    wrong_id["id"] = reasoning2["id"]
    cases.append(("reasoning_wrong_id", replay_payload(model, wrong_id, call1, normal_output)))
    wrong_payload = copy.deepcopy(reasoning1)
    wrong_payload["encrypted_content"] = reasoning2["encrypted_content"]
    cases.append(
        ("reasoning_wrong_payload", replay_payload(model, wrong_payload, call1, normal_output))
    )
    cases.append(
        ("reasoning_from_other_pair", replay_payload(model, reasoning2, call1, normal_output))
    )

    truncated = copy.deepcopy(reasoning1)
    truncated["encrypted_content"] = truncated["encrypted_content"][:-1]
    cases.append(("reasoning_truncated_one", replay_payload(model, truncated, call1, normal_output)))
    truncated_half = copy.deepcopy(reasoning1)
    encrypted = truncated_half["encrypted_content"]
    truncated_half["encrypted_content"] = encrypted[: len(encrypted) // 2]
    cases.append(
        ("reasoning_truncated_half", replay_payload(model, truncated_half, call1, normal_output))
    )

    wrong_call_item_id = copy.deepcopy(call1)
    wrong_call_item_id["id"] = f"fc_{uuid.uuid4().hex}"
    cases.append(
        (
            "function_call_wrong_item_id",
            replay_payload(model, reasoning1, wrong_call_item_id, normal_output),
        )
    )
    wrong_call_id = copy.deepcopy(call1)
    wrong_call_id["call_id"] = f"call_{uuid.uuid4().hex}"
    cases.append(
        ("function_call_wrong_call_id", replay_payload(model, reasoning1, wrong_call_id, normal_output))
    )
    wrong_arguments = copy.deepcopy(call1)
    wrong_arguments["arguments"] = json.dumps({"text": "mutated"}, separators=(",", ":"))
    cases.append(
        ("function_call_wrong_arguments", replay_payload(model, reasoning1, wrong_arguments, normal_output))
    )

    duplicate = replay_payload(model, reasoning1, call1, normal_output)
    duplicate["input"].insert(1, copy.deepcopy(reasoning1))
    cases.append(("duplicate_reasoning", duplicate))

    added = stream1.get("added_reasoning")
    done = stream1.get("done_reasoning")
    if isinstance(added, dict):
        if isinstance(added.get("encrypted_content"), str):
            cases.append(
                ("added_snapshot", replay_payload(model, added, call1, normal_output))
            )
        added_with_terminal_payload = copy.deepcopy(added)
        added_with_terminal_payload["encrypted_content"] = reasoning1["encrypted_content"]
        cases.append(
            (
                "added_id_terminal_payload",
                replay_payload(model, added_with_terminal_payload, call1, normal_output),
            )
        )
    if isinstance(done, dict) and isinstance(done.get("encrypted_content"), str):
        cases.append(("done_snapshot", replay_payload(model, done, call1, normal_output)))

    if actual_capture is not None:
        actual = copy.deepcopy(actual_capture["attempts"][0]["upstream_request"])
        actual["stream"] = True
        actual_reasoning_positions = [
            index
            for index, item in enumerate(actual["input"])
            if isinstance(item, dict)
            and item.get("type") == "reasoning"
            and isinstance(item.get("encrypted_content"), str)
        ]
        actual_last_position = actual_reasoning_positions[-1]
        actual_last = copy.deepcopy(actual["input"][actual_last_position])
        cases.append(("actual_full", copy.deepcopy(actual)))

        value = copy.deepcopy(actual)
        for item in value["input"]:
            if isinstance(item, dict) and item.get("type") == "reasoning":
                item.pop("encrypted_content", None)
        cases.append(("actual_all_no_encrypted", value))

        value = copy.deepcopy(actual)
        value["input"] = [
            item
            for item in value["input"]
            if not (isinstance(item, dict) and item.get("type") == "reasoning")
        ]
        cases.append(("actual_all_reasoning_removed", value))

        value = copy.deepcopy(actual)
        value["input"][actual_last_position].pop("encrypted_content", None)
        cases.append(("actual_last_no_encrypted", value))

        value = copy.deepcopy(actual)
        value["input"].pop(actual_last_position)
        cases.append(("actual_last_removed", value))

        value = copy.deepcopy(actual)
        value["input"][actual_last_position] = copy.deepcopy(reasoning1)
        cases.append(("actual_last_replaced_fresh", value))

        value = copy.deepcopy(actual)
        value["input"][actual_last_position]["encrypted_content"] = reasoning1[
            "encrypted_content"
        ]
        cases.append(("actual_last_fresh_payload_old_id", value))

        value = copy.deepcopy(actual)
        value["input"][actual_last_position]["id"] = reasoning1["id"]
        cases.append(("actual_last_fresh_id_old_payload", value))

        value = copy.deepcopy(actual)
        value["input"][actual_last_position]["status"] = "completed"
        cases.append(("actual_last_add_completed_status", value))

        value = copy.deepcopy(actual)
        value["input"][actual_last_position] = copy.deepcopy(actual_last)
        value["input"][actual_last_position]["content"] = []
        value["input"][actual_last_position]["summary"] = []
        cases.append(("actual_last_without_summary", value))

    selected_cases = {
        name.strip()
        for name in os.environ.get("CPA_FUZZ_CASES", "").split(",")
        if name.strip()
    }
    for name, payload in cases:
        if selected_cases and name not in selected_cases:
            continue
        run_case(url, api_key, name, payload)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
