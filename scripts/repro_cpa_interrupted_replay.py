#!/usr/bin/env python3
"""Reproduce CPA replay failures from a fresh interrupted upstream stream."""

from __future__ import annotations

import argparse
import copy
import hashlib
import http.client
import json
import os
import socket
import ssl
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from dataclasses import dataclass
from typing import Any


REQUEST_TIMEOUT_SECONDS = 180


@dataclass
class HttpResult:
    status: int
    body: bytes
    headers: dict[str, str]
    elapsed_seconds: float


@dataclass
class CaptureResult:
    cutoff_event: str
    request_id: str | None
    response_id: str | None
    saw_terminal: bool
    reasoning_added: dict[str, Any] | None
    reasoning_done: dict[str, Any] | None
    function_call: dict[str, Any]
    event_log_tail: list[str]
    elapsed_seconds: float


@dataclass
class AssistantCaptureResult:
    request_id: str | None
    response_id: str | None
    saw_terminal: bool
    reasoning_added: dict[str, Any] | None
    reasoning_done: dict[str, Any] | None
    partial_output_text: str
    event_log_tail: list[str]
    elapsed_seconds: float


def normalize_responses_url(url: str) -> str:
    value = url.rstrip("/")
    if value.endswith("/responses"):
        return value
    if value.endswith("/v1"):
        return value + "/responses"
    return value + "/v1/responses"


def resolve_api_key(explicit: str | None) -> str:
    if explicit:
        return explicit
    for name in ("CPA_API_KEY", "MONOIZE_PROBE_API_KEY"):
        value = os.environ.get(name)
        if value:
            return value
    raise SystemExit("missing API key: set CPA_API_KEY or MONOIZE_PROBE_API_KEY")


def resolve_url(explicit: str | None) -> str:
    value = explicit or os.environ.get("CPA_RESPONSES_URL")
    if not value:
        raise SystemExit("missing responses URL: set CPA_RESPONSES_URL")
    return normalize_responses_url(value)


def make_generation_payload(model: str, marker: str, padding_repeats: int) -> dict[str, Any]:
    padding = "|".join([marker] * padding_repeats)
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
                            "Then call the echo tool exactly once. "
                            "The tool argument text must be a single string in this exact format: "
                            f"'verdict=<yes-or-no>;marker={marker};padding={padding}'. "
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


def make_assistant_generation_payload(model: str, marker: str) -> dict[str, Any]:
    return {
        "model": model,
        "input": [
            {
                "type": "message",
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": (
                            "Think carefully about whether 'tacocat' is a palindrome. "
                            f"Include the marker {marker} exactly once in the final answer. "
                            "Answer in one short sentence."
                        ),
                    }
                ],
            }
        ],
        "reasoning": {"effort": "high", "summary": "auto"},
        "include": ["reasoning.encrypted_content"],
        "max_output_tokens": 256,
        "stream": True,
        "store": False,
    }


def sanitize_reasoning(item: dict[str, Any]) -> dict[str, Any]:
    value = copy.deepcopy(item)
    value.pop("status", None)
    value.pop("started_at", None)
    value.pop("duration", None)
    return value


def sanitize_function_call(item: dict[str, Any]) -> dict[str, Any]:
    value = copy.deepcopy(item)
    value.pop("status", None)
    return value


def make_replay_payload(
    model: str,
    reasoning: dict[str, Any],
    function_call: dict[str, Any],
    user_text: str,
    include_function_output: bool,
    include_tools: bool,
) -> dict[str, Any]:
    call = sanitize_function_call(function_call)
    replay_input: list[dict[str, Any]] = [sanitize_reasoning(reasoning), call]
    if include_function_output:
        replay_input.append(
            {
                "type": "function_call_output",
                "call_id": call["call_id"],
                "output": (
                    "Skipped because a new user message interrupted the in-flight tool turn."
                ),
            }
        )
    replay_input.append(
        {
            "role": "user",
            "content": [{"type": "input_text", "text": user_text}],
        }
    )
    payload = {
        "model": model,
        "input": replay_input,
        "reasoning": {"effort": "low", "summary": "auto"},
        "include": ["reasoning.encrypted_content"],
        "max_output_tokens": 256,
        "stream": True,
        "store": False,
    }
    if include_tools:
        payload["tools"] = make_generation_payload(model, "unused", 1)["tools"]
    return payload


def make_control_payload(
    model: str,
    function_call: dict[str, Any],
    user_text: str,
) -> dict[str, Any]:
    call = sanitize_function_call(function_call)
    return {
        "model": model,
        "input": [
            call,
            {
                "type": "function_call_output",
                "call_id": call["call_id"],
                "output": "Skipped because a new user message interrupted the in-flight tool turn.",
            },
            {
                "role": "user",
                "content": [{"type": "input_text", "text": user_text}],
            },
        ],
        "tools": make_generation_payload(model, "unused", 1)["tools"],
        "reasoning": {"effort": "low", "summary": "auto"},
        "include": ["reasoning.encrypted_content"],
        "max_output_tokens": 256,
        "stream": True,
        "store": False,
    }


def make_assistant_replay_payload(
    model: str,
    reasoning: dict[str, Any],
    assistant_text: str,
    user_text: str,
    include_reasoning: bool,
) -> dict[str, Any]:
    replay_input: list[dict[str, Any]] = []
    if include_reasoning:
        replay_input.append(sanitize_reasoning(reasoning))
    replay_input.extend(
        [
            {
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": assistant_text}],
            },
            {
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": user_text}],
            },
        ]
    )
    return {
        "model": model,
        "input": replay_input,
        "reasoning": {"effort": "low", "summary": "auto"},
        "include": ["reasoning.encrypted_content"],
        "max_output_tokens": 128,
        "stream": True,
        "store": False,
    }


def make_mixed_reasoning(
    capture_added: dict[str, Any] | None,
    capture_done: dict[str, Any] | None,
    prefix_source: str,
    cut_ratio: float,
) -> tuple[dict[str, Any], dict[str, Any]]:
    added = copy.deepcopy(choose_reasoning(capture_added, capture_done, "added"))
    done = copy.deepcopy(choose_reasoning(capture_added, capture_done, "done"))
    added_enc = added["encrypted_content"]
    done_enc = done["encrypted_content"]
    if not isinstance(added_enc, str) or not isinstance(done_enc, str):
        raise RuntimeError("mixed-token scenario requires string encrypted_content")
    cut = int(min(len(added_enc), len(done_enc)) * cut_ratio)
    mixed = copy.deepcopy(done)
    if prefix_source == "added":
        mixed["encrypted_content"] = added_enc[:cut] + done_enc[cut:]
    elif prefix_source == "done":
        mixed["encrypted_content"] = done_enc[:cut] + added_enc[cut:]
    else:
        raise RuntimeError(f"unsupported prefix source: {prefix_source}")
    return mixed, {
        "prefix_source": prefix_source,
        "cut": cut,
        "cut_ratio": cut_ratio,
        "added_len": len(added_enc),
        "done_len": len(done_enc),
    }


def post_buffered(url: str, api_key: str, payload: dict[str, Any]) -> HttpResult:
    body = json.dumps(payload, separators=(",", ":")).encode()
    request = urllib.request.Request(
        url,
        data=body,
        method="POST",
        headers={
            "Authorization": f"Bearer {api_key}",
            "Content-Type": "application/json",
            "User-Agent": "reqwest",
        },
    )
    started = time.monotonic()
    try:
        with urllib.request.urlopen(request, timeout=REQUEST_TIMEOUT_SECONDS) as response:
            return HttpResult(
                status=response.status,
                body=response.read(),
                headers={key.lower(): value for key, value in response.headers.items()},
                elapsed_seconds=time.monotonic() - started,
            )
    except urllib.error.HTTPError as error:
        return HttpResult(
            status=error.code,
            body=error.read(),
            headers={key.lower(): value for key, value in error.headers.items()},
            elapsed_seconds=time.monotonic() - started,
        )


def parse_sse_events(body: bytes) -> list[dict[str, Any]]:
    events: list[dict[str, Any]] = []
    event_name: str | None = None
    data_lines: list[str] = []
    for raw_line in body.splitlines():
        line = raw_line.decode(errors="replace").rstrip("\r\n")
        if line.startswith("event:"):
            event_name = line[6:].strip()
            continue
        if line.startswith("data:"):
            data_lines.append(line[5:].strip())
            continue
        if line or not data_lines:
            continue
        raw = "\n".join(data_lines)
        event_name_local = event_name
        event_name = None
        data_lines = []
        if raw == "[DONE]":
            continue
        try:
            data = json.loads(raw)
        except json.JSONDecodeError:
            continue
        if event_name_local and "type" not in data:
            data["type"] = event_name_local
        events.append(data)
    return events


def classify_result(result: HttpResult) -> str:
    text = result.body.decode(errors="replace").lower()
    if result.status >= 500 or "response.failed" in text:
        return "failed"
    if result.status >= 400:
        return "rejected"
    if "response.completed" in text or "response.incomplete" in text:
        return "accepted"
    return "unknown"


def body_preview(body: bytes, limit: int = 320) -> str:
    text = body.decode(errors="replace").strip()
    if len(text) <= limit:
        return text
    return text[:limit] + "...(truncated)"


def summarize_failure(body: bytes) -> dict[str, Any] | None:
    failure = next(
        (event for event in parse_sse_events(body) if event.get("type") == "response.failed"),
        None,
    )
    if not isinstance(failure, dict):
        return None
    response = failure.get("response")
    if not isinstance(response, dict):
        return None
    error = response.get("error")
    if not isinstance(error, dict):
        return None
    return {
        "response_id": response.get("id"),
        "error_code": error.get("code"),
        "error_message": error.get("message"),
    }


def item_fingerprint(item: dict[str, Any]) -> dict[str, Any]:
    encrypted = item.get("encrypted_content")
    arguments = item.get("arguments")
    return {
        "type": item.get("type"),
        "id": item.get("id"),
        "call_id": item.get("call_id"),
        "name": item.get("name"),
        "encrypted_chars": len(encrypted) if isinstance(encrypted, str) else None,
        "encrypted_sha256": (
            hashlib.sha256(encrypted.encode()).hexdigest()
            if isinstance(encrypted, str)
            else None
        ),
        "arguments_chars": len(arguments) if isinstance(arguments, str) else None,
        "arguments_sha256": (
            hashlib.sha256(arguments.encode()).hexdigest()
            if isinstance(arguments, str)
            else None
        ),
    }


def choose_reasoning(
    capture_added: dict[str, Any] | None,
    capture_done: dict[str, Any] | None,
    source: str,
) -> dict[str, Any]:
    if source == "added":
        if not isinstance(capture_added, dict):
            raise RuntimeError("requested reasoning.added but the stream did not expose one")
        return capture_added
    if source == "done":
        if not isinstance(capture_done, dict):
            raise RuntimeError("requested reasoning.done but the stream did not expose one")
        return capture_done
    raise RuntimeError(f"unsupported reasoning source: {source}")


def open_stream(url: str, api_key: str, payload: dict[str, Any]) -> tuple[http.client.HTTPConnection, http.client.HTTPResponse]:
    parsed = urllib.parse.urlsplit(url)
    scheme = parsed.scheme.lower()
    host = parsed.hostname
    if not host:
        raise RuntimeError(f"invalid URL: {url}")
    port = parsed.port or (443 if scheme == "https" else 80)
    path = parsed.path or "/"
    if parsed.query:
        path += "?" + parsed.query
    if scheme == "https":
        context = ssl.create_default_context()
        connection: http.client.HTTPConnection = http.client.HTTPSConnection(
            host,
            port,
            timeout=REQUEST_TIMEOUT_SECONDS,
            context=context,
        )
    elif scheme == "http":
        connection = http.client.HTTPConnection(host, port, timeout=REQUEST_TIMEOUT_SECONDS)
    else:
        raise RuntimeError(f"unsupported URL scheme: {scheme}")
    body = json.dumps(payload, separators=(",", ":")).encode()
    connection.request(
        "POST",
        path,
        body=body,
        headers={
            "Authorization": f"Bearer {api_key}",
            "Content-Type": "application/json",
            "User-Agent": "reqwest",
            "Accept": "text/event-stream",
        },
    )
    response = connection.getresponse()
    return connection, response


def capture_until_cutoff(
    url: str,
    api_key: str,
    payload: dict[str, Any],
    cutoff_event: str,
) -> CaptureResult:
    connection, response = open_stream(url, api_key, payload)
    started = time.monotonic()
    request_id = response.getheader("x-request-id")
    reasoning_added: dict[str, Any] | None = None
    reasoning_done: dict[str, Any] | None = None
    current_call: dict[str, Any] | None = None
    response_id: str | None = None
    saw_terminal = False
    event_name: str | None = None
    data_lines: list[str] = []
    event_log_tail: list[str] = []
    try:
        while True:
            raw_line = response.readline()
            if not raw_line:
                raise RuntimeError("stream ended before cutoff event")
            line = raw_line.decode(errors="replace").rstrip("\r\n")
            if line.startswith("event:"):
                event_name = line[6:].strip()
                continue
            if line.startswith("data:"):
                data_lines.append(line[5:].strip())
                continue
            if line or not data_lines:
                continue
            raw = "\n".join(data_lines)
            data_lines = []
            name = event_name
            event_name = None
            if raw == "[DONE]":
                continue
            try:
                data = json.loads(raw)
            except json.JSONDecodeError:
                continue
            kind = name or str(data.get("type") or "")
            event_log_tail.append(kind)
            event_log_tail = event_log_tail[-10:]
            if kind == "response.created":
                response_id = data.get("response", {}).get("id") or response_id
            if kind == "response.output_item.added":
                item = data.get("item")
                if isinstance(item, dict):
                    if item.get("type") == "reasoning":
                        reasoning_added = copy.deepcopy(item)
                    elif item.get("type") == "function_call":
                        current_call = copy.deepcopy(item)
            elif kind == "response.function_call_arguments.delta":
                if not isinstance(current_call, dict):
                    current_call = {"type": "function_call"}
                current_call["arguments"] = str(current_call.get("arguments") or "") + str(
                    data.get("delta") or ""
                )
            elif kind == "response.function_call_arguments.done":
                if not isinstance(current_call, dict):
                    current_call = {"type": "function_call"}
                if "arguments" in data:
                    current_call["arguments"] = data["arguments"]
                if "call_id" in data:
                    current_call["call_id"] = data["call_id"]
                if "item_id" in data:
                    current_call["id"] = data["item_id"]
            elif kind == "response.output_item.done":
                item = data.get("item")
                if isinstance(item, dict):
                    if item.get("type") == "reasoning":
                        reasoning_done = copy.deepcopy(item)
                    elif item.get("type") == "function_call":
                        current_call = copy.deepcopy(item)
            elif kind in {"response.completed", "response.failed", "response.incomplete"}:
                saw_terminal = True
            if kind == cutoff_event:
                if not isinstance(reasoning_added, dict) and not isinstance(reasoning_done, dict):
                    raise RuntimeError(f"cutoff {cutoff_event} reached before reasoning item")
                if not isinstance(current_call, dict):
                    raise RuntimeError(f"cutoff {cutoff_event} reached before function_call item")
                return CaptureResult(
                    cutoff_event=cutoff_event,
                    request_id=request_id,
                    response_id=response_id,
                    saw_terminal=saw_terminal,
                    reasoning_added=reasoning_added,
                    reasoning_done=reasoning_done,
                    function_call=current_call,
                    event_log_tail=event_log_tail,
                    elapsed_seconds=time.monotonic() - started,
                )
    except (socket.timeout, TimeoutError) as error:
        raise RuntimeError(f"stream timed out before cutoff {cutoff_event}") from error
    finally:
        try:
            response.close()
        finally:
            connection.close()


def capture_assistant_until_output(
    url: str,
    api_key: str,
    payload: dict[str, Any],
    min_output_chars: int,
) -> AssistantCaptureResult:
    connection, response = open_stream(url, api_key, payload)
    started = time.monotonic()
    request_id = response.getheader("x-request-id")
    reasoning_added: dict[str, Any] | None = None
    reasoning_done: dict[str, Any] | None = None
    response_id: str | None = None
    saw_terminal = False
    event_name: str | None = None
    data_lines: list[str] = []
    event_log_tail: list[str] = []
    output_text = ""
    try:
        while True:
            raw_line = response.readline()
            if not raw_line:
                raise RuntimeError("assistant stream ended before output cutoff")
            line = raw_line.decode(errors="replace").rstrip("\r\n")
            if line.startswith("event:"):
                event_name = line[6:].strip()
                continue
            if line.startswith("data:"):
                data_lines.append(line[5:].strip())
                continue
            if line or not data_lines:
                continue
            raw = "\n".join(data_lines)
            data_lines = []
            name = event_name
            event_name = None
            if raw == "[DONE]":
                continue
            try:
                data = json.loads(raw)
            except json.JSONDecodeError:
                continue
            kind = name or str(data.get("type") or "")
            event_log_tail.append(kind)
            event_log_tail = event_log_tail[-10:]
            if kind == "response.created":
                response_id = data.get("response", {}).get("id") or response_id
            elif kind == "response.output_item.added":
                item = data.get("item")
                if isinstance(item, dict) and item.get("type") == "reasoning":
                    reasoning_added = copy.deepcopy(item)
            elif kind == "response.output_item.done":
                item = data.get("item")
                if isinstance(item, dict) and item.get("type") == "reasoning":
                    reasoning_done = copy.deepcopy(item)
            elif kind == "response.output_text.delta":
                output_text += str(data.get("delta") or "")
                if (
                    (isinstance(reasoning_added, dict) or isinstance(reasoning_done, dict))
                    and len(output_text) >= min_output_chars
                ):
                    return AssistantCaptureResult(
                        request_id=request_id,
                        response_id=response_id,
                        saw_terminal=saw_terminal,
                        reasoning_added=reasoning_added,
                        reasoning_done=reasoning_done,
                        partial_output_text=output_text,
                        event_log_tail=event_log_tail,
                        elapsed_seconds=time.monotonic() - started,
                    )
            elif kind in {"response.completed", "response.failed", "response.incomplete"}:
                saw_terminal = True
    except (socket.timeout, TimeoutError) as error:
        raise RuntimeError("assistant stream timed out before output cutoff") from error
    finally:
        try:
            response.close()
        finally:
            connection.close()


def run_round(
    url: str,
    api_key: str,
    model: str,
    cutoff_event: str,
    padding_repeats: int,
    user_text: str,
    print_payloads: bool,
) -> tuple[dict[str, Any], bool]:
    marker = uuid.uuid4().hex
    capture = capture_until_cutoff(
        url,
        api_key,
        make_generation_payload(model, marker, padding_repeats),
        cutoff_event,
    )
    replay_payloads: dict[str, dict[str, Any]] = {}
    for source in ("added", "done"):
        try:
            reasoning_item = choose_reasoning(
                capture.reasoning_added,
                capture.reasoning_done,
                source,
            )
        except RuntimeError:
            continue
        for include_function_output in (True, False):
            for include_tools in (True, False):
                name = (
                    f"{source}_"
                    f"{'with_output' if include_function_output else 'without_output'}_"
                    f"{'with_tools' if include_tools else 'without_tools'}"
                )
                replay_payloads[name] = make_replay_payload(
                    model,
                    reasoning_item,
                    capture.function_call,
                    user_text,
                    include_function_output=include_function_output,
                    include_tools=include_tools,
                )
    control_payload = make_control_payload(model, capture.function_call, user_text)
    replay_results = {
        name: post_buffered(url, api_key, payload)
        for name, payload in replay_payloads.items()
    }
    control = post_buffered(url, api_key, control_payload)
    replay_outcomes = {
        name: classify_result(result) for name, result in replay_results.items()
    }
    control_outcome = classify_result(control)
    passed = (
        not capture.saw_terminal
        and any(outcome == "failed" for outcome in replay_outcomes.values())
        and control_outcome == "accepted"
    )
    summary = {
        "marker": marker,
        "capture": {
            "cutoff_event": capture.cutoff_event,
            "request_id": capture.request_id,
            "response_id": capture.response_id,
            "elapsed_seconds": round(capture.elapsed_seconds, 3),
            "saw_terminal_before_abort": capture.saw_terminal,
            "event_log_tail": capture.event_log_tail,
            "reasoning_added": item_fingerprint(capture.reasoning_added)
            if isinstance(capture.reasoning_added, dict)
            else None,
            "reasoning_done": item_fingerprint(capture.reasoning_done)
            if isinstance(capture.reasoning_done, dict)
            else None,
            "function_call": item_fingerprint(capture.function_call),
        },
        "replays": {
            name: {
                "http_status": result.status,
                "outcome": replay_outcomes[name],
                "elapsed_seconds": round(result.elapsed_seconds, 3),
                "request_id": result.headers.get("x-request-id"),
                "failure": summarize_failure(result.body),
                "body_preview": body_preview(result.body)
                if replay_outcomes[name] != "accepted"
                else None,
            }
            for name, result in replay_results.items()
        },
        "control": {
            "http_status": control.status,
            "outcome": control_outcome,
            "elapsed_seconds": round(control.elapsed_seconds, 3),
            "request_id": control.headers.get("x-request-id"),
            "failure": summarize_failure(control.body),
            "body_preview": body_preview(control.body) if control_outcome != "accepted" else None,
        },
        "pass": passed,
    }
    print(json.dumps({"round": summary}, separators=(",", ":")), flush=True)
    if print_payloads:
        print(
            json.dumps(
                {
                    "payloads": {
                        "replays": replay_payloads,
                        "control": control_payload,
                    }
                },
                separators=(",", ":"),
            ),
            flush=True,
        )
    return summary, passed


def run_assistant_round(
    url: str,
    api_key: str,
    model: str,
    user_text: str,
    print_payloads: bool,
    min_output_chars: int,
    max_attempts: int,
) -> tuple[dict[str, Any], bool]:
    last_error: Exception | None = None
    marker = ""
    capture: AssistantCaptureResult | None = None
    attempt = 0
    for attempt in range(1, max_attempts + 1):
        marker = uuid.uuid4().hex
        try:
            capture = capture_assistant_until_output(
                url,
                api_key,
                make_assistant_generation_payload(model, marker),
                min_output_chars,
            )
            break
        except RuntimeError as error:
            last_error = error
    if capture is None:
        raise RuntimeError(
            f"assistant scenario did not reach output cutoff after {max_attempts} attempts: {last_error}"
        )
    fake_output = f"It is a palindrome. marker={marker}"
    replay_payloads: dict[str, dict[str, Any]] = {}
    for source in ("added", "done"):
        try:
            reasoning_item = choose_reasoning(
                capture.reasoning_added,
                capture.reasoning_done,
                source,
            )
        except RuntimeError:
            continue
        replay_payloads[source] = make_assistant_replay_payload(
            model,
            reasoning_item,
            fake_output,
            user_text,
            include_reasoning=True,
        )
    control_payload = make_assistant_replay_payload(
        model,
        choose_reasoning(capture.reasoning_added, capture.reasoning_done, "done")
        if isinstance(capture.reasoning_done, dict)
        else choose_reasoning(capture.reasoning_added, capture.reasoning_done, "added"),
        fake_output,
        user_text,
        include_reasoning=False,
    )
    replay_results = {
        name: post_buffered(url, api_key, payload)
        for name, payload in replay_payloads.items()
    }
    control = post_buffered(url, api_key, control_payload)
    replay_outcomes = {
        name: classify_result(result) for name, result in replay_results.items()
    }
    control_outcome = classify_result(control)
    passed = (
        not capture.saw_terminal
        and any(outcome == "failed" for outcome in replay_outcomes.values())
        and control_outcome == "accepted"
    )
    summary = {
        "marker": marker,
        "capture": {
            "attempt": attempt,
            "request_id": capture.request_id,
            "response_id": capture.response_id,
            "elapsed_seconds": round(capture.elapsed_seconds, 3),
            "saw_terminal_before_abort": capture.saw_terminal,
            "event_log_tail": capture.event_log_tail,
            "partial_output_text": capture.partial_output_text,
            "reasoning_added": item_fingerprint(capture.reasoning_added)
            if isinstance(capture.reasoning_added, dict)
            else None,
            "reasoning_done": item_fingerprint(capture.reasoning_done)
            if isinstance(capture.reasoning_done, dict)
            else None,
        },
        "replays": {
            name: {
                "http_status": result.status,
                "outcome": replay_outcomes[name],
                "elapsed_seconds": round(result.elapsed_seconds, 3),
                "request_id": result.headers.get("x-request-id"),
                "failure": summarize_failure(result.body),
                "body_preview": body_preview(result.body)
                if replay_outcomes[name] != "accepted"
                else None,
            }
            for name, result in replay_results.items()
        },
        "control": {
            "http_status": control.status,
            "outcome": control_outcome,
            "elapsed_seconds": round(control.elapsed_seconds, 3),
            "request_id": control.headers.get("x-request-id"),
            "failure": summarize_failure(control.body),
            "body_preview": body_preview(control.body) if control_outcome != "accepted" else None,
        },
        "pass": passed,
    }
    print(
        json.dumps({"assistant_round": summary}, separators=(",", ":")),
        flush=True,
    )
    if print_payloads:
        print(
            json.dumps(
                {
                    "assistant_payloads": {
                        "replays": replay_payloads,
                        "control": control_payload,
                    }
                },
                separators=(",", ":"),
            ),
            flush=True,
        )
    return summary, passed


def run_mixed_token_round(
    url: str,
    api_key: str,
    model: str,
    user_text: str,
    print_payloads: bool,
    padding_repeats: int,
    compare_url: str | None,
    compare_api_key: str | None,
    prefix_source: str,
    cut_ratio: float,
) -> tuple[dict[str, Any], bool]:
    marker = uuid.uuid4().hex
    capture = capture_until_cutoff(
        url,
        api_key,
        make_generation_payload(model, marker, padding_repeats),
        "response.function_call_arguments.done",
    )
    reasoning, splice = make_mixed_reasoning(
        capture.reasoning_added,
        capture.reasoning_done,
        prefix_source,
        cut_ratio,
    )
    payload = make_replay_payload(
        model,
        reasoning,
        capture.function_call,
        user_text,
        include_function_output=True,
        include_tools=True,
    )
    targets: list[tuple[str, str, str]] = [("primary", url, api_key)]
    if compare_url and compare_api_key:
        targets.append(("compare", normalize_responses_url(compare_url), compare_api_key))
    results = {}
    for name, target_url, target_key in targets:
        result = post_buffered(target_url, target_key, payload)
        outcome = classify_result(result)
        results[name] = {
            "http_status": result.status,
            "outcome": outcome,
            "elapsed_seconds": round(result.elapsed_seconds, 3),
            "request_id": result.headers.get("x-request-id"),
            "failure": summarize_failure(result.body),
            "body_preview": body_preview(result.body),
        }
    primary = results["primary"]
    passed = primary["outcome"] in {"rejected", "failed"}
    summary = {
        "marker": marker,
        "capture": {
            "response_id": capture.response_id,
            "elapsed_seconds": round(capture.elapsed_seconds, 3),
            "reasoning_added": item_fingerprint(capture.reasoning_added)
            if isinstance(capture.reasoning_added, dict)
            else None,
            "reasoning_done": item_fingerprint(capture.reasoning_done)
            if isinstance(capture.reasoning_done, dict)
            else None,
            "function_call": item_fingerprint(capture.function_call),
        },
        "splice": splice,
        "results": results,
        "pass": passed,
    }
    print(json.dumps({"mixed_round": summary}, separators=(",", ":")), flush=True)
    if print_payloads:
        print(
            json.dumps({"mixed_payloads": {"payload": payload}}, separators=(",", ":")),
            flush=True,
        )
    return summary, passed


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Create fresh CPA reasoning/tool streams, abort them midstream, and replay "
            "the visible history back to CPA."
        )
    )
    parser.add_argument("--url", help="Responses endpoint URL. Defaults to CPA_RESPONSES_URL.")
    parser.add_argument("--api-key", help="Bearer token. Defaults to CPA_API_KEY or MONOIZE_PROBE_API_KEY.")
    parser.add_argument("--model", default=os.environ.get("CPA_MODEL", "gpt-5.6-sol"))
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument(
        "--scenario",
        choices=("tool-call", "assistant-output", "mixed-token"),
        default="tool-call",
    )
    parser.add_argument(
        "--cutoff-event",
        default="response.function_call_arguments.done",
        help="Abort immediately after this SSE event is observed.",
    )
    parser.add_argument("--padding-repeats", type=int, default=96)
    parser.add_argument("--assistant-min-output-chars", type=int, default=12)
    parser.add_argument("--assistant-max-attempts", type=int, default=4)
    parser.add_argument("--compare-url", help="Optional second Responses endpoint for side-by-side replay.")
    parser.add_argument("--compare-api-key", help="API key for --compare-url.")
    parser.add_argument(
        "--mixed-prefix-source",
        choices=("added", "done"),
        default="added",
    )
    parser.add_argument("--mixed-cut-ratio", type=float, default=0.5)
    parser.add_argument(
        "--user-text",
        default="The previous tool turn was interrupted. Reply with OK only.",
    )
    parser.add_argument(
        "--print-payloads",
        action="store_true",
        help="Print the exact replay/control JSON bodies for each round.",
    )
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    url = resolve_url(args.url)
    api_key = resolve_api_key(args.api_key)
    all_passed = True
    for index in range(args.rounds):
        try:
            if args.scenario == "tool-call":
                _, passed = run_round(
                    url=url,
                    api_key=api_key,
                    model=args.model,
                    cutoff_event=args.cutoff_event,
                    padding_repeats=args.padding_repeats,
                    user_text=args.user_text,
                    print_payloads=args.print_payloads,
                )
            else:
                if args.scenario == "assistant-output":
                    _, passed = run_assistant_round(
                        url=url,
                        api_key=api_key,
                        model=args.model,
                        user_text=args.user_text,
                        print_payloads=args.print_payloads,
                        min_output_chars=args.assistant_min_output_chars,
                        max_attempts=args.assistant_max_attempts,
                    )
                else:
                    _, passed = run_mixed_token_round(
                        url=url,
                        api_key=api_key,
                        model=args.model,
                        user_text=args.user_text,
                        print_payloads=args.print_payloads,
                        padding_repeats=args.padding_repeats,
                        compare_url=args.compare_url,
                        compare_api_key=args.compare_api_key,
                        prefix_source=args.mixed_prefix_source,
                        cut_ratio=args.mixed_cut_ratio,
                    )
        except Exception as error:  # noqa: BLE001
            all_passed = False
            print(
                json.dumps(
                    {
                        "round": {
                            "index": index,
                            "pass": False,
                            "error": str(error),
                        }
                    },
                    separators=(",", ":"),
                ),
                flush=True,
            )
            continue
        all_passed = all_passed and passed
    return 0 if all_passed else 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
