#!/usr/bin/env python3
"""Hermess plugin wrapper for the SkillHub FTShare-market-data skill."""

from __future__ import annotations

import json
import os
from pathlib import Path
import re
import runpy
import sys
import traceback
from typing import Any
from datetime import datetime, timezone, timedelta
import contextlib
import io


PLUGIN_DIR = Path(__file__).resolve().parent
REPO_ROOT = PLUGIN_DIR.parent.parent
SKILL_DIR = REPO_ROOT / ".hermess" / "skills" / "ftshare-market-data"
SKILL_RUN = SKILL_DIR / "run.py"
BEIJING_TZ = timezone(timedelta(hours=8))

INDEX_ALIASES = {
    "上证指数": "000001.XSHG",
    "上证综指": "000001.XSHG",
    "上海综指": "000001.XSHG",
    "沪指": "000001.XSHG",
    "深证成指": "399001.XSHE",
    "深成指": "399001.XSHE",
    "创业板指": "399006.XSHE",
    "沪深300": "000300.XSHG",
    "中证500": "000905.XSHG",
    "上证50": "000016.XSHG",
    "科创50": "000688.XSHG",
}


def _load_payload() -> dict[str, Any]:
    raw = sys.stdin.read().strip()
    if not raw:
        return {}
    try:
        value = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise SystemExit(f"Invalid JSON input: {exc}") from exc
    if not isinstance(value, dict):
        raise SystemExit("Input must be a JSON object")
    return value


def _available_subskills() -> list[str]:
    subskills_dir = SKILL_DIR / "sub-skills"
    if not subskills_dir.is_dir():
        return []
    names = []
    for path in subskills_dir.iterdir():
        if (path / "scripts" / "handler.py").is_file():
            names.append(path.name)
    return sorted(names)


def _flag_name(key: str) -> str:
    return "--" + key.strip()


def _args_to_argv(args: dict[str, Any]) -> list[str]:
    argv: list[str] = []
    for key, value in args.items():
        if value is None or value is False:
            continue
        flag = _flag_name(key)
        if value is True:
            argv.append(flag)
        elif isinstance(value, list):
            for item in value:
                argv.extend([flag, str(item)])
        else:
            argv.extend([flag, str(value)])
    return argv


def _span_from_query(query: str) -> str | None:
    if any(word in query for word in ("年线", "年K", "年度")):
        return "YEAR1"
    if any(word in query for word in ("月线", "月K", "月度")):
        return "MONTH1"
    if any(word in query for word in ("周线", "周K", "周度")):
        return "WEEK1"
    if any(word in query for word in ("日线", "日K", "日 k", "K线", "k线", "开高低收")):
        return "DAY1"
    return None


def _date_to_until_ms(query: str) -> int | None:
    match = re.search(r"(?<!\d)(20\d{2})([01]\d)([0-3]\d)(?!\d)", query)
    if not match:
        match = re.search(r"(?<!\d)(20\d{2})[-/.年]([01]?\d)[-/.月]([0-3]?\d)日?(?!\d)", query)
    if not match:
        return None
    year, month, day = (int(v) for v in match.groups())
    dt = datetime(year, month, day, 23, 59, 59, tzinfo=BEIJING_TZ)
    return int(dt.timestamp() * 1000)


def _value_to_until_ms(value: Any) -> int | None:
    if value is None:
        return None
    text = str(value).strip()
    if not text:
        return None
    if re.fullmatch(r"\d{13}", text):
        return int(text)
    return _date_to_until_ms(text)


def _index_from_query(query: str) -> str | None:
    for alias, code in INDEX_ALIASES.items():
        if alias in query:
            return code
    match = re.search(r"(?<!\d)(\d{6})\.(XSHG|XSHE|BJ)(?!\w)", query, re.IGNORECASE)
    if match and any(word in query for word in ("指数", "指", "index", "Index")):
        return f"{match.group(1)}.{match.group(2).upper()}"
    return None


def _stock_from_query(query: str) -> str | None:
    match = re.search(r"(?<!\d)(\d{6})\.(XSHG|XSHE|SZ|SH|BJ)(?!\w)", query, re.IGNORECASE)
    if not match:
        return None
    code, suffix = match.group(1), match.group(2).upper()
    suffix = {"SH": "XSHG", "SZ": "SZ"}.get(suffix, suffix)
    return f"{code}.{suffix}"


def _repair_args_from_query(subskill: str, args: dict[str, Any], query: str) -> tuple[str, dict[str, Any]]:
    repaired = {
        key: value
        for key, value in args.items()
        if not key.startswith("_") and key not in {"date", "--date"}
    }
    for date_key in ("date", "--date"):
        until_ms = _value_to_until_ms(args.get(date_key))
        if until_ms is not None:
            repaired.setdefault("until_ts_ms", until_ms)
            repaired.setdefault("limit", 1)

    if not query:
        if subskill in {"index-ohlcs", "stock-ohlcs"}:
            repaired.setdefault("span", "DAY1")
        return subskill, repaired

    wants_kline = any(word in query for word in ("K线", "k线", "日线", "周线", "月线", "年线", "开高低收"))
    index_code = _index_from_query(query)

    if wants_kline and index_code and subskill in {"stock-ohlcs", "index-ohlcs"}:
        subskill = "index-ohlcs"
        repaired.pop("stock", None)
        repaired.setdefault("index", index_code)

    if subskill == "index-ohlcs":
        if index_code:
            repaired.setdefault("index", index_code)
        repaired.setdefault("span", _span_from_query(query) or "DAY1")
        until_ms = _date_to_until_ms(query)
        if until_ms is not None:
            repaired.setdefault("until_ts_ms", until_ms)
            repaired.setdefault("limit", 1)
        else:
            repaired.setdefault("limit", 50)

    if subskill == "stock-ohlcs":
        stock_code = _stock_from_query(query)
        if stock_code:
            repaired.setdefault("stock", stock_code)
        repaired.setdefault("span", _span_from_query(query) or "DAY1")
        until_ms = _date_to_until_ms(query)
        if until_ms is not None:
            repaired.setdefault("until_ts_ms", until_ms)
            repaired.setdefault("limit", 1)
        else:
            repaired.setdefault("limit", 50)

    return subskill, repaired


def _argv_to_args(argv: list[str]) -> dict[str, Any]:
    args: dict[str, Any] = {}
    i = 0
    while i < len(argv):
        token = argv[i]
        if not token.startswith("--"):
            i += 1
            continue
        key = token[2:]
        if i + 1 >= len(argv) or argv[i + 1].startswith("--"):
            args[key] = True
            i += 1
        else:
            args[key] = argv[i + 1]
            i += 2
    return args


def _normalize_argv(subskill: str, argv: list[str], query: str) -> tuple[str, list[str]]:
    args = _argv_to_args(argv)
    repaired_subskill, repaired_args = _repair_args_from_query(subskill, args, query)
    return repaired_subskill, _args_to_argv(repaired_args)


def _execute_skill(subskill: str, argv: list[str]) -> int:
    if not SKILL_RUN.is_file():
        raise SystemExit(f"FTShare skill package is missing: {SKILL_RUN}")
    allowed = set(_available_subskills())
    if subskill not in allowed:
        raise SystemExit(
            "Unknown FTShare subskill: "
            f"{subskill}. Available subskills: {', '.join(sorted(allowed))}"
        )

    old_argv = sys.argv[:]
    old_cwd = os.getcwd()
    stdout_buf = io.StringIO()
    stderr_buf = io.StringIO()
    try:
        os.chdir(SKILL_DIR)
        sys.argv = [str(SKILL_RUN), subskill, *argv]
        with contextlib.redirect_stdout(stdout_buf), contextlib.redirect_stderr(stderr_buf):
            runpy.run_path(str(SKILL_RUN), run_name="__main__")
        print(stdout_buf.getvalue(), end="")
        return 0
    except SystemExit as exc:
        code = exc.code if isinstance(exc.code, int) else 1
        stdout = stdout_buf.getvalue()
        stderr = stderr_buf.getvalue()
        if code == 0:
            print(stdout, end="")
        else:
            print(
                json.dumps(
                    {
                        "error": "FTShare skill execution failed",
                        "subskill": subskill,
                        "argv": argv,
                        "exit_code": code,
                        "stderr": stderr.strip(),
                    },
                    ensure_ascii=False,
                    indent=2,
                ),
                file=sys.stderr,
            )
        return code
    except Exception as exc:
        print(
            json.dumps(
                {
                    "error": "FTShare skill execution failed",
                    "subskill": subskill,
                    "argv": argv,
                    "message": str(exc),
                    "exception": exc.__class__.__name__,
                    "stderr": stderr_buf.getvalue().strip(),
                },
                ensure_ascii=False,
                indent=2,
            ),
            file=sys.stderr,
        )
        if os.environ.get("HERMESS_FTSHARE_DEBUG"):
            traceback.print_exc(file=sys.stderr)
        return 1
    finally:
        sys.argv = old_argv
        os.chdir(old_cwd)


def main() -> int:
    payload = _load_payload()
    if payload.get("list_subskills"):
        print(json.dumps({"subskills": _available_subskills()}, ensure_ascii=False, indent=2))
        return 0

    subskill = str(payload.get("subskill") or "").strip()
    if not subskill:
        raise SystemExit("Missing required field: subskill")
    query = str(payload.get("query") or payload.get("natural_query") or "").strip()

    raw_argv = payload.get("argv")
    if raw_argv is not None:
        if not isinstance(raw_argv, list) or not all(isinstance(v, str) for v in raw_argv):
            raise SystemExit("argv must be an array of strings")
        subskill, argv = _normalize_argv(subskill, raw_argv, query)
    else:
        raw_args = payload.get("args") or {}
        if not isinstance(raw_args, dict):
            raise SystemExit("args must be an object")
        subskill, raw_args = _repair_args_from_query(subskill, raw_args, query)
        argv = _args_to_argv(raw_args)

    return _execute_skill(subskill, argv)


if __name__ == "__main__":
    raise SystemExit(main())
