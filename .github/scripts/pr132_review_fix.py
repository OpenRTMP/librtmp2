from __future__ import annotations

import re
from pathlib import Path

path = Path("src/client/mod.rs")
text = path.read_text()
pattern = re.compile(
    r"(    fn drain_ready_messages_rejects_oversized_multitrack_video\(\) \{"
    r".*?        cmsg\.msg_stream_id = 1;\n)"
    r"        chunk_write\(&mut wire, &cmsg, &payload, payload\.len\(\), 128\)\.unwrap\(\);\n\n"
    r"        let mut client = Client::new\(\);\n",
    re.DOTALL,
)
replacement = (
    r"\1"
    "        let chunk_size = payload.len();\n"
    "        chunk_write(&mut wire, &cmsg, &payload, payload.len(), chunk_size).unwrap();\n\n"
    "        let mut client = Client::new();\n"
    "        client.chunk_reg.set_all_chunk_size(chunk_size as u32);\n"
)
updated, count = pattern.subn(replacement, text, count=1)
if count != 1:
    raise RuntimeError(f"expected one oversized-multitrack test block, found {count}")
path.write_text(updated)
