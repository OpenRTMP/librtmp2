from pathlib import Path
import runpy

runpy.run_path("scripts/apply_pr128_fixes.py", run_name="__main__")

path = Path("src/server/mod.rs")
text = path.read_text()
old = '''            frame_type,
            timestamp: 0,
            payload,
'''
new = '''            frame_type,
            timestamp: 0,
            cache_payload: payload.clone(),
            payload,
'''
if text.count(old) != 1:
    raise RuntimeError(f"expected one RelayFrame test-helper initializer, found {text.count(old)}")
path.write_text(text.replace(old, new, 1))
print("Applied RelayFrame test-helper follow-up")
