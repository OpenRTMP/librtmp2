from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    file_path = Path(path)
    text = file_path.read_text()
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f"expected exactly one follow-up match in {path}, got {count}")
    file_path.write_text(text.replace(old, new, 1))


replace_once(
    "src/server/mod.rs",
    "            cache_payload: payload.clone(),\n            payload,",
    "            cache_payload: None,\n            payload,",
)

replace_once(
    "src/session/conn.rs",
    """        assert_eq!(
            conn.pending_relay[0].cache_payload,
            vec![0x91, b'a', b'v', b'c', b'1', 0, 0, 0, 0xAA]
        );""",
    """        assert_eq!(
            conn.pending_relay[0].cache_payload(),
            &[0x91, b'a', b'v', b'c', b'1', 0, 0, 0, 0xAA]
        );""",
)

Path(__file__).unlink()
