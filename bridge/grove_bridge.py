#!/usr/bin/env python3

from __future__ import annotations

import asyncio
import json
import os
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Protocol

try:
    import iterm2  # type: ignore
except ImportError:  # pragma: no cover - unavailable in unit tests
    iterm2 = None


ROLE_GROVE = "grove"
ROLE_AI = "ai"
ROLE_EDITOR = "editor"
VALID_ROLES = {ROLE_GROVE, ROLE_AI, ROLE_EDITOR}
PICKER_TARGET_MARKERS = (
    "aider",
    "claude",
    "claude-code",
    "codex",
    "helix",
    "hx",
    "nvim",
    "vim",
    "zed",
)
BRIDGE_DEBUG_CONFIG_RELATIVE_PATH = Path(".config/grove/bridge-debug.json")


@dataclass(frozen=True)
class BridgeDebugConfig:
    path: Path
    log_session_lists: bool = True


class BridgeLogger:
    def __init__(self, config: BridgeDebugConfig | None = None) -> None:
        self._config = config
        self._handle = None
        self._write_failed = False
        if config is None:
            return
        config.path.parent.mkdir(parents=True, exist_ok=True)
        self._handle = config.path.open("a", encoding="utf-8")

    @property
    def enabled(self) -> bool:
        return self._handle is not None and not self._write_failed

    @property
    def log_session_lists(self) -> bool:
        return self.enabled and self._config is not None and self._config.log_session_lists

    @property
    def path(self) -> Path | None:
        if self._config is None:
            return None
        return self._config.path

    def log(self, event: str, **fields: Any) -> None:
        if not self.enabled:
            return
        record = {"ts_ms": current_timestamp_ms(), "event": event, **fields}
        try:
            assert self._handle is not None
            self._handle.write(json.dumps(record, sort_keys=True, separators=(",", ":")))
            self._handle.write("\n")
            self._handle.flush()
        except OSError as exc:
            self._write_failed = True
            print(
                f"grove bridge debug logging disabled after write failure: {exc}",
                file=sys.stderr,
            )

    def close(self) -> None:
        if self._handle is None:
            return
        self._handle.close()
        self._handle = None


@dataclass
class SessionLocationHint:
    window_id: str | None = None
    tab_id: str | None = None
    window_title: str | None = None
    tab_title: str | None = None

    def to_dict(self) -> dict[str, Any]:
        return {
            "window_id": self.window_id,
            "tab_id": self.tab_id,
            "window_title": self.window_title,
            "tab_title": self.tab_title,
        }


@dataclass
class BridgeSession:
    session_id: str
    title: str
    role: str | None = None
    job_name: str | None = None
    command_line: str | None = None
    cwd: str | None = None
    location_hint: SessionLocationHint | None = None
    instance_id: str | None = None

    def normalized_role(self) -> str | None:
        if not self.role:
            return None
        return self.role

    def same_tab_as(self, other: "BridgeSession") -> bool:
        if not self.location_hint or not other.location_hint:
            return False
        return (
            self.location_hint.window_id == other.location_hint.window_id
            and self.location_hint.tab_id == other.location_hint.tab_id
        )

    def same_window_as(self, other: "BridgeSession") -> bool:
        if not self.location_hint or not other.location_hint:
            return False
        return self.location_hint.window_id == other.location_hint.window_id

    def to_summary(self) -> dict[str, Any]:
        return {
            "session_id": self.session_id,
            "title": self.title,
            "role": self.normalized_role(),
            "job_name": self.job_name,
            "command_line": self.command_line,
            "cwd": self.cwd,
            "location_hint": self.location_hint.to_dict() if self.location_hint else None,
        }


class SessionStore(Protocol):
    async def list_sessions(self) -> list[BridgeSession]:
        ...

    async def set_role(self, session_id: str, role: str | None) -> None:
        ...

    async def clear_role(self, session_id: str) -> None:
        ...

    async def send_text(self, session_id: str, text: str) -> None:
        ...


class InMemorySessionStore:
    def __init__(self, sessions: list[BridgeSession]) -> None:
        self._sessions = {session.session_id: session for session in sessions}
        self.sent_text: list[tuple[str, str]] = []

    async def list_sessions(self) -> list[BridgeSession]:
        return list(self._sessions.values())

    async def set_role(self, session_id: str, role: str | None) -> None:
        self.get_session(session_id).role = role

    async def clear_role(self, session_id: str) -> None:
        self.get_session(session_id).role = None

    async def send_text(self, session_id: str, text: str) -> None:
        self.get_session(session_id)
        self.sent_text.append((session_id, text))

    def get_session(self, session_id: str) -> BridgeSession:
        if session_id not in self._sessions:
            raise KeyError(f"unknown session_id: {session_id}")
        return self._sessions[session_id]


class BridgeController:
    def __init__(self, store: SessionStore, logger: BridgeLogger | None = None) -> None:
        self._store = store
        self._logger = logger or BridgeLogger()

    async def handle_envelope(self, envelope: dict[str, Any]) -> dict[str, Any]:
        request_id = str(envelope.get("request_id", ""))
        command_name = "unknown"
        try:
            command_name, payload = decode_command(envelope.get("command"))
            self._logger.log(
                "bridge.command_received",
                request_id=request_id,
                command=command_name,
            )
            response = await self._handle_command(command_name, payload, request_id)
        except Exception as exc:  # pragma: no cover - defensive transport guard
            self._logger.log(
                "bridge.command_error",
                request_id=request_id,
                command=command_name,
                error=str(exc),
            )
            response = {"error": {"message": str(exc)}}
        return {"request_id": request_id, "response": response}

    async def _handle_command(
        self, command_name: str, payload: dict[str, Any], request_id: str
    ) -> dict[str, Any] | str:
        if command_name == "ping":
            return "pong"

        if command_name == "list_sessions":
            instance_id = str(payload["instance_id"])
            sessions = await self._store.list_sessions()
            sender = find_sender_session(sessions, instance_id)
            self._logger.log(
                "bridge.sender_resolved",
                request_id=request_id,
                command=command_name,
                instance_id=instance_id,
                session_id=sender.session_id,
            )
            summaries: list[dict[str, Any]] = []
            for session in sessions:
                include, reason = picker_visibility_decision(session, sender)
                if self._logger.log_session_lists:
                    self._logger.log(
                        "bridge.list_sessions_decision",
                        request_id=request_id,
                        session_id=session.session_id,
                        decision="include" if include else "exclude",
                        reason=reason,
                    )
                if include:
                    summaries.append(session_picker_summary(session))
            return {"session_list": summaries}

        if command_name == "set_role":
            session_id = str(payload["session_id"])
            role = validate_role(payload["role"])
            await self._set_role(session_id, role, request_id)
            return "pong"

        if command_name == "clear_role":
            session_id = str(payload["session_id"])
            await self._store.clear_role(session_id)
            self._logger.log(
                "bridge.clear_role",
                request_id=request_id,
                session_id=session_id,
            )
            return "pong"

        if command_name == "resolve_targets":
            instance_id = str(payload["instance_id"])
            resolution = await self._resolve_targets(instance_id, request_id)
            return {"targets_resolved": resolution}

        if command_name == "send_text":
            return await self._send_text(payload, request_id)

        if command_name == "get_session_snapshot":
            return {"error": {"message": "get_session_snapshot is not implemented"}}

        return {"error": {"message": f"unsupported bridge command: {command_name}"}}

    async def _set_role(self, session_id: str, role: str, request_id: str) -> None:
        sessions = await self._store.list_sessions()
        target = find_session_by_id(sessions, session_id)
        window_id = (
            target.location_hint.window_id
            if target.location_hint and target.location_hint.window_id
            else None
        )
        cleared_session_ids: list[str] = []
        if window_id is not None:
            for session in sessions:
                if (
                    session.session_id != target.session_id
                    and session.normalized_role() == role
                    and session.location_hint
                    and session.location_hint.window_id == window_id
                ):
                    await self._store.clear_role(session.session_id)
                    cleared_session_ids.append(session.session_id)
        await self._store.set_role(session_id, role)
        self._logger.log(
            "bridge.set_role",
            request_id=request_id,
            session_id=session_id,
            role=role,
            cleared_session_ids=cleared_session_ids,
        )

    async def _resolve_targets(self, instance_id: str, request_id: str) -> dict[str, Any]:
        sessions = await self._store.list_sessions()
        sender = find_sender_session(sessions, instance_id)
        self._logger.log(
            "bridge.sender_resolved",
            request_id=request_id,
            command="resolve_targets",
            instance_id=instance_id,
            session_id=sender.session_id,
        )

        ai_target, ai_source = resolve_role_target(sessions, sender, ROLE_AI)
        editor_target, editor_source = resolve_role_target(sessions, sender, ROLE_EDITOR)

        source = "same_tab"
        if ai_source == "same_window" or editor_source == "same_window":
            source = "same_window"

        resolution = {
            "ai_target_session_id": ai_target.session_id if ai_target else None,
            "editor_target_session_id": editor_target.session_id if editor_target else None,
            "source": source,
        }
        self._logger.log(
            "bridge.resolve_targets",
            request_id=request_id,
            ai_target_session_id=resolution["ai_target_session_id"],
            editor_target_session_id=resolution["editor_target_session_id"],
            source=source,
        )
        return resolution

    async def _send_text(self, payload: dict[str, Any], request_id: str) -> dict[str, Any]:
        instance_id = str(payload["instance_id"])
        target = payload["target"]
        text = str(payload["text"])
        if payload.get("append_newline"):
            text = f"{text}\n"

        sessions = await self._store.list_sessions()
        sender = find_sender_session(sessions, instance_id)
        self._logger.log(
            "bridge.sender_resolved",
            request_id=request_id,
            command="send_text",
            instance_id=instance_id,
            session_id=sender.session_id,
        )
        target_session: BridgeSession | None = None

        if isinstance(target, dict) and "role" in target:
            role = validate_role(target["role"])
            target_session, _ = resolve_role_target(sessions, sender, role)
            if target_session is None:
                self._logger.log(
                    "bridge.send_text_manual_selection_required",
                    request_id=request_id,
                    role=role,
                    text_length=len(text),
                )
                return {"manual_selection_required": {"role": role}}
        elif isinstance(target, dict) and "session_id" in target:
            target_session = find_session_by_id(sessions, str(target["session_id"]))
        else:
            raise ValueError("send_text target must be a role or session_id object")

        await self._store.send_text(target_session.session_id, text)
        self._logger.log(
            "bridge.send_text",
            request_id=request_id,
            target_session_id=target_session.session_id,
            text_length=len(text),
        )
        return {"send_ok": {"target_session_id": target_session.session_id}}


class GroveBridgeServer:
    def __init__(
        self, controller: BridgeController, socket_path: Path | None = None
    ) -> None:
        self._controller = controller
        self.socket_path = socket_path or default_socket_path()
        self._server: asyncio.AbstractServer | None = None

    async def start(self) -> None:
        if self.socket_path.exists():
            self.socket_path.unlink()
        self._server = await asyncio.start_unix_server(
            self._handle_client, path=str(self.socket_path)
        )

    async def close(self) -> None:
        if self._server is not None:
            self._server.close()
            await self._server.wait_closed()
            self._server = None
        if self.socket_path.exists():
            self.socket_path.unlink()

    async def serve_forever(self) -> None:
        await self.start()
        assert self._server is not None
        async with self._server:
            await self._server.serve_forever()

    async def _handle_client(
        self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter
    ) -> None:
        try:
            while True:
                line = await reader.readline()
                if not line:
                    break
                envelope = json.loads(line.decode("utf-8"))
                response = await self._controller.handle_envelope(envelope)
                writer.write(json.dumps(response).encode("utf-8"))
                writer.write(b"\n")
                await writer.drain()
        finally:
            writer.close()
            await writer.wait_closed()


class Iterm2SessionStore:
    def __init__(self, connection: Any) -> None:
        self._connection = connection

    async def list_sessions(self) -> list[BridgeSession]:
        app = await iterm2.async_get_app(self._connection)
        sessions: list[BridgeSession] = []
        for window in app.windows:
            for tab in window.tabs:
                tab_title = await safe_async_get_variable(tab, "title")
                for session in tab.all_sessions:
                    role = await normalize_session_variable(session, "user.groveRole")
                    instance_id = await normalize_session_variable(
                        session, "user.groveInstance"
                    )
                    title = await safe_session_title(session)
                    job_name = await normalize_session_variable(session, "jobName")
                    command_line = await normalize_session_variable(session, "commandLine")
                    cwd = await normalize_session_variable(session, "path")
                    sessions.append(
                        BridgeSession(
                            session_id=session.session_id,
                            title=title or session.session_id,
                            role=role,
                            job_name=job_name,
                            command_line=command_line,
                            cwd=cwd,
                            location_hint=SessionLocationHint(
                                window_id=window.window_id,
                                tab_id=tab.tab_id,
                                window_title=None,
                                tab_title=tab_title,
                            ),
                            instance_id=instance_id,
                        )
                    )
        return sessions

    async def set_role(self, session_id: str, role: str | None) -> None:
        session = await self._require_session(session_id)
        await session.async_set_variable("user.groveRole", role or "")

    async def clear_role(self, session_id: str) -> None:
        await self.set_role(session_id, None)

    async def send_text(self, session_id: str, text: str) -> None:
        session = await self._require_session(session_id)
        await session.async_send_text(text, suppress_broadcast=True)

    async def _require_session(self, session_id: str) -> Any:
        app = await iterm2.async_get_app(self._connection)
        session = app.get_session_by_id(session_id)
        if session is None:
            raise ValueError(f"unknown session_id: {session_id}")
        return session


def default_socket_path() -> Path:
    tmp_root = Path(os.environ.get("TMPDIR", "/tmp"))
    return tmp_root / f"grove-bridge-{os.getuid()}.sock"


def default_bridge_debug_config_path() -> Path:
    return Path.home() / BRIDGE_DEBUG_CONFIG_RELATIVE_PATH


def load_bridge_debug_config(config_path: Path | None = None) -> BridgeDebugConfig | None:
    resolved_path = config_path or default_bridge_debug_config_path()
    if not resolved_path.exists():
        return None
    data = json.loads(resolved_path.read_text(encoding="utf-8"))
    if not isinstance(data, dict):
        raise ValueError("bridge debug config must be a JSON object")
    log_path = data.get("path")
    if not isinstance(log_path, str) or not log_path.strip():
        raise ValueError("bridge debug config requires a non-empty string path")
    log_session_lists = data.get("log_session_lists", True)
    if not isinstance(log_session_lists, bool):
        raise ValueError("bridge debug config log_session_lists must be a boolean")
    return BridgeDebugConfig(
        path=Path(log_path).expanduser(),
        log_session_lists=log_session_lists,
    )


def build_bridge_logger(config_path: Path | None = None) -> BridgeLogger:
    resolved_path = config_path or default_bridge_debug_config_path()
    try:
        config = load_bridge_debug_config(resolved_path)
    except ValueError as exc:
        raise SystemExit(
            f"invalid Grove bridge debug config at {resolved_path}: {exc}"
        ) from exc
    logger = BridgeLogger(config)
    if logger.enabled:
        assert logger.path is not None
        logger.log(
            "bridge.config_loaded",
            config_path=str(resolved_path),
            log_path=str(logger.path),
            log_session_lists=logger.log_session_lists,
        )
    return logger


def decode_command(command: Any) -> tuple[str, dict[str, Any]]:
    if isinstance(command, str):
        return command, {}
    if isinstance(command, dict) and len(command) == 1:
        command_name, payload = next(iter(command.items()))
        if payload is None:
            payload = {}
        if not isinstance(payload, dict):
            raise ValueError("bridge command payload must be an object")
        return str(command_name), payload
    raise ValueError("bridge command must be a string or single-key object")


def validate_role(role: Any) -> str:
    if role not in VALID_ROLES:
        raise ValueError(f"invalid grove role: {role}")
    return str(role)


def find_session_by_id(
    sessions: list[BridgeSession], session_id: str
) -> BridgeSession:
    for session in sessions:
        if session.session_id == session_id:
            return session
    raise ValueError(f"unknown session_id: {session_id}")


def find_sender_session(
    sessions: list[BridgeSession], instance_id: str
) -> BridgeSession:
    for session in sessions:
        if session.instance_id == instance_id:
            return session
    raise ValueError(f"unable to find grove session for instance_id: {instance_id}")


def session_picker_summary(session: BridgeSession) -> dict[str, Any]:
    summary = session.to_summary()
    if session_has_stale_grove_target_markers(session):
        summary["role"] = None
    return summary


def picker_visibility_decision(
    session: BridgeSession, sender: BridgeSession
) -> tuple[bool, str]:
    if session.session_id == sender.session_id:
        return False, "sender_session"
    if session_is_live_grove(session):
        return False, "live_grove_session"
    if session_has_stale_grove_target_markers(session):
        return True, "stale_grove_target_markers"
    return True, "eligible_session"


def session_is_live_grove(session: BridgeSession) -> bool:
    return (
        session.normalized_role() == ROLE_GROVE
        and not session_has_stale_grove_target_markers(session)
    )


def session_has_stale_grove_target_markers(session: BridgeSession) -> bool:
    if session.normalized_role() != ROLE_GROVE:
        return False
    return session_fields_contain_any_marker(
        (session.job_name, session.command_line),
        PICKER_TARGET_MARKERS,
    )


def session_fields_contain_any_marker(
    values: tuple[str | None, ...], markers: tuple[str, ...]
) -> bool:
    for value in values:
        if value is None:
            continue
        lowered = value.lower()
        if any(marker in lowered for marker in markers):
            return True
    return False


def resolve_role_target(
    sessions: list[BridgeSession], sender: BridgeSession, role: str
) -> tuple[BridgeSession | None, str | None]:
    same_tab = [
        session
        for session in sessions
        if session.session_id != sender.session_id
        and session.normalized_role() == role
        and session.same_tab_as(sender)
    ]
    if same_tab:
        return same_tab[0], "same_tab"

    same_window = [
        session
        for session in sessions
        if session.session_id != sender.session_id
        and session.normalized_role() == role
        and session.same_window_as(sender)
    ]
    if same_window:
        return same_window[0], "same_window"

    return None, None


async def safe_async_get_variable(scope: Any, name: str) -> str | None:
    value = await scope.async_get_variable(name)
    if value in ("", None):
        return None
    return str(value)


async def normalize_session_variable(session: Any, name: str) -> str | None:
    return await safe_async_get_variable(session, name)


async def safe_session_title(session: Any) -> str | None:
    title = await safe_async_get_variable(session, "presentationName")
    if title:
        return title
    return await safe_async_get_variable(session, "name")


def current_timestamp_ms() -> int:
    return time.time_ns() // 1_000_000


async def main(connection: Any) -> None:  # pragma: no cover - exercised in iTerm2
    logger = build_bridge_logger()
    controller = BridgeController(Iterm2SessionStore(connection), logger=logger)
    server = GroveBridgeServer(controller)
    try:
        await server.start()
        logger.log("bridge.startup", socket_path=str(server.socket_path))
        await asyncio.Future()
    finally:
        logger.log("bridge.shutdown", socket_path=str(server.socket_path))
        logger.close()
        await server.close()


if __name__ == "__main__":  # pragma: no cover - exercised manually in iTerm2
    if iterm2 is None:
        raise SystemExit(
            "The iTerm2 Python API module is unavailable. Run this from iTerm2 AutoLaunch "
            "or an environment where the iTerm2 Python package is installed."
        )
    iterm2.run_forever(main)
