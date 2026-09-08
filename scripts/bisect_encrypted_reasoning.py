#!/usr/bin/env python3
"""Minimize encrypted-reasoning sets from a Monoize request capture."""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
import re
import sys
import time
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable


UPSTREAM_REQUEST_ID = re.compile(
    r"request ID ([0-9a-f-]{36})|(?:x-request-id|request_id)[\"':= ]+([0-9a-f-]{36})",
    re.IGNORECASE,
)


@dataclass(frozen=True)
class ProbeResult:
    selected: tuple[int, ...]
    http_status: int
    outcome: str
    elapsed_seconds: float
    upstream_request_id: str | None
    response_bytes: int

    def as_json(self) -> dict[str, object]:
        return {
            "selected_count": len(self.selected),
            "selected": list(self.selected),
            "http_status": self.http_status,
            "outcome": self.outcome,
            "elapsed_seconds": round(self.elapsed_seconds, 3),
            "upstream_request_id": self.upstream_request_id,
            "response_bytes": self.response_bytes,
        }


def load_request(
    path: Path, attempt: int, request_field: str
) -> tuple[dict[str, object], list[int]]:
    capture = json.loads(sys.stdin.read()) if str(path) == "-" else json.loads(path.read_text())
    attempts = capture.get("attempts")
    if not isinstance(attempts, list) or attempt < 1 or attempt > len(attempts):
        raise ValueError(f"capture has no attempt {attempt}")
    request = attempts[attempt - 1].get(request_field)
    if not isinstance(request, dict) or not isinstance(request.get("input"), list):
        raise ValueError(
            f"capture attempt has no object {request_field} with an input array"
        )

    positions = [
        position
        for position, item in enumerate(request["input"])
        if isinstance(item, dict)
        and item.get("type") == "reasoning"
        and isinstance(item.get("encrypted_content"), str)
    ]
    if not positions:
        raise ValueError("request has no encrypted reasoning items")
    return request, positions


def describe_tokens(request: dict[str, object], positions: list[int]) -> None:
    inputs = request["input"]
    assert isinstance(inputs, list)
    for ordinal, position in enumerate(positions):
        item = inputs[position]
        assert isinstance(item, dict)
        encrypted = item["encrypted_content"]
        assert isinstance(encrypted, str)
        print(
            json.dumps(
                {
                    "ordinal": ordinal,
                    "input_position": position,
                    "item_id": item.get("id"),
                    "encrypted_chars": len(encrypted),
                    "encrypted_sha256": hashlib.sha256(encrypted.encode()).hexdigest(),
                },
                separators=(",", ":"),
            )
        )


def build_payload(
    request: dict[str, object], positions: list[int], selected: Iterable[int]
) -> dict[str, object]:
    selected_set = set(selected)
    position_to_ordinal = {position: ordinal for ordinal, position in enumerate(positions)}
    payload = copy.deepcopy(request)
    inputs = payload["input"]
    assert isinstance(inputs, list)
    payload["input"] = [
        item
        for position, item in enumerate(inputs)
        if position not in position_to_ordinal
        or position_to_ordinal[position] in selected_set
    ]
    payload["max_output_tokens"] = 16
    payload["stream"] = True
    return payload


def build_isolated_payload(
    request: dict[str, object],
    positions: list[int],
    ordinal: int,
    context: str,
    preserve_top_level: bool,
) -> dict[str, object]:
    inputs = request["input"]
    assert isinstance(inputs, list)
    reasoning_item = copy.deepcopy(inputs[positions[ordinal]])
    if context == "fake-output":
        isolated_input = [
            reasoning_item,
            {
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "Acknowledged."}],
            },
        ]
    elif context == "reasoning-only":
        isolated_input = [reasoning_item]
    elif context == "original-output":
        source_position = positions[ordinal]
        if source_position + 1 >= len(inputs):
            raise ValueError(f"reasoning ordinal {ordinal} has no following output item")
        isolated_input = [reasoning_item, copy.deepcopy(inputs[source_position + 1])]
        output_item = inputs[source_position + 1]
        if (
            isinstance(output_item, dict)
            and output_item.get("type") == "function_call"
            and source_position + 2 < len(inputs)
        ):
            candidate = inputs[source_position + 2]
            if (
                isinstance(candidate, dict)
                and candidate.get("type") == "function_call_output"
                and candidate.get("call_id") == output_item.get("call_id")
            ):
                isolated_input.append(copy.deepcopy(candidate))
    else:
        raise ValueError(f"unsupported isolated context: {context}")
    isolated_input.append(
        {
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "Reply with OK."}],
        }
    )
    if preserve_top_level:
        payload = copy.deepcopy(request)
        payload["input"] = isolated_input
    else:
        payload = {
            "model": request["model"],
            "input": isolated_input,
            "store": False,
        }
        for key in ("include", "reasoning"):
            if key in request:
                payload[key] = copy.deepcopy(request[key])
    payload["max_output_tokens"] = 16
    payload["stream"] = True
    return payload


def classify_response(status: int, body: bytes) -> tuple[str, str | None]:
    text = body.decode("utf-8", errors="replace")
    match = UPSTREAM_REQUEST_ID.search(text)
    request_id = next((group for group in match.groups() if group), None) if match else None
    lowered = text.lower()
    if status >= 500 or "response.failed" in lowered or "upstream status 500" in lowered:
        return "failed", request_id
    if status >= 400:
        return "http_error", request_id
    if "response.completed" in lowered or "response.incomplete" in lowered:
        return "accepted", request_id
    return "unknown", request_id


def probe(
    *,
    url: str,
    api_key: str,
    request: dict[str, object],
    positions: list[int],
    selected: Iterable[int],
    timeout: float,
) -> ProbeResult:
    selected_tuple = tuple(sorted(selected))
    payload = build_payload(request, positions, selected_tuple)
    encoded = json.dumps(payload, separators=(",", ":")).encode()
    req = urllib.request.Request(
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
        with urllib.request.urlopen(req, timeout=timeout) as response:
            status = response.status
            body = response.read()
    except urllib.error.HTTPError as error:
        status = error.code
        body = error.read()
    outcome, upstream_request_id = classify_response(status, body)
    return ProbeResult(
        selected=selected_tuple,
        http_status=status,
        outcome=outcome,
        elapsed_seconds=time.monotonic() - started,
        upstream_request_id=upstream_request_id,
        response_bytes=len(body),
    )


def probe_payload(
    *, url: str, api_key: str, payload: dict[str, object], ordinal: int, timeout: float
) -> ProbeResult:
    encoded = json.dumps(payload, separators=(",", ":")).encode()
    req = urllib.request.Request(
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
        with urllib.request.urlopen(req, timeout=timeout) as response:
            status = response.status
            body = response.read()
    except urllib.error.HTTPError as error:
        status = error.code
        body = error.read()
    outcome, upstream_request_id = classify_response(status, body)
    return ProbeResult(
        selected=(ordinal,),
        http_status=status,
        outcome=outcome,
        elapsed_seconds=time.monotonic() - started,
        upstream_request_id=upstream_request_id,
        response_bytes=len(body),
    )


def emit(label: str, result: ProbeResult) -> None:
    row = result.as_json()
    row["label"] = label
    print(json.dumps(row, separators=(",", ":")), flush=True)


def find_boundary(
    *,
    direction: str,
    count: int,
    run,
) -> int | None:
    low = 1
    high = count
    first_failure: int | None = None
    while low <= high:
        size = (low + high) // 2
        selected = range(size) if direction == "prefix" else range(count - size, count)
        result = run(f"{direction}:{size}", selected)
        if result.outcome == "failed":
            first_failure = size
            high = size - 1
        elif result.outcome == "accepted":
            low = size + 1
        else:
            raise RuntimeError(
                f"cannot continue {direction} boundary search after {result.outcome}"
            )
    return first_failure


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("capture", type=Path)
    parser.add_argument("--attempt", type=int, default=1)
    parser.add_argument(
        "--request-field",
        choices=("raw_input", "upstream_request"),
        default="raw_input",
    )
    parser.add_argument("--url", default="http://127.0.0.1:40550/v1/responses")
    parser.add_argument("--api-key-env", default="MONOIZE_PROBE_API_KEY")
    parser.add_argument("--timeout", type=float, default=180.0)
    parser.add_argument(
        "--strategy",
        choices=("describe", "boundary", "isolated", "leave-one-out"),
        default="boundary",
    )
    parser.add_argument(
        "--ordinals",
        help="comma-separated encrypted-reasoning ordinals; isolated strategy only",
    )
    parser.add_argument("--concurrency", type=int, default=4)
    parser.add_argument(
        "--isolated-context",
        choices=("fake-output", "reasoning-only", "original-output"),
        default="fake-output",
    )
    parser.add_argument("--preserve-top-level", action="store_true")
    args = parser.parse_args()

    request, positions = load_request(args.capture, args.attempt, args.request_field)
    if args.strategy == "describe":
        describe_tokens(request, positions)
        return 0

    api_key = os.environ.get(args.api_key_env)
    if not api_key:
        print(f"missing API key environment variable: {args.api_key_env}", file=sys.stderr)
        return 2

    cache: dict[tuple[int, ...], ProbeResult] = {}

    def run(label: str, selected: Iterable[int]) -> ProbeResult:
        key = tuple(sorted(selected))
        if key not in cache:
            cache[key] = probe(
                url=args.url,
                api_key=api_key,
                request=request,
                positions=positions,
                selected=key,
                timeout=args.timeout,
            )
        emit(label, cache[key])
        return cache[key]

    count = len(positions)
    if args.strategy == "isolated":
        if args.ordinals:
            ordinals = [int(value) for value in args.ordinals.split(",")]
        else:
            ordinals = list(range(count))
        if any(ordinal < 0 or ordinal >= count for ordinal in ordinals):
            raise ValueError(f"ordinals must be between 0 and {count - 1}")
        with ThreadPoolExecutor(max_workers=args.concurrency) as executor:
            futures = {
                executor.submit(
                    probe_payload,
                    url=args.url,
                    api_key=api_key,
                    payload=build_isolated_payload(
                        request,
                        positions,
                        ordinal,
                        args.isolated_context,
                        args.preserve_top_level,
                    ),
                    ordinal=ordinal,
                    timeout=args.timeout,
                ): ordinal
                for ordinal in ordinals
            }
            for future in as_completed(futures):
                ordinal = futures[future]
                emit(f"isolated:{ordinal}", future.result())
        return 0

    if args.strategy == "leave-one-out":
        baseline = run("all", range(count))
        if baseline.outcome != "failed":
            raise RuntimeError("full encrypted-reasoning set did not reproduce the failure")
        for omitted in range(count):
            run(f"without:{omitted}", (index for index in range(count) if index != omitted))
        return 0


    baseline = run("all", range(count))
    if baseline.outcome != "failed":
        raise RuntimeError("full encrypted-reasoning set did not reproduce the failure")
    prefix = find_boundary(direction="prefix", count=count, run=run)
    suffix = find_boundary(direction="suffix", count=count, run=run)
    print(
        json.dumps(
            {
                "summary": {
                    "encrypted_reasoning_count": count,
                    "first_failing_prefix_size": prefix,
                    "first_failing_suffix_size": suffix,
                }
            },
            separators=(",", ":"),
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
