from pathlib import Path
import runpy

runpy.run_path("scripts/apply_pr128_followup.py", run_name="__main__")

path = Path("src/ertmp/connect_amf.rs")
text = path.read_text()
old = '''        let n = amf0::read_string(buf, &mut cc)?;
        if n >= 4 {
            fourcc_list_add(list, &cc[..n])?;
        }
'''
new = '''        let n = amf0::read_string(buf, &mut cc)?;
        if (n == 1 && cc[0] == b'*') || n >= 4 {
            fourcc_list_add(list, &cc[..n])?;
        }
'''
if text.count(old) != 1:
    raise RuntimeError(f"expected one strict-array FourCC parser, found {text.count(old)}")
path.write_text(text.replace(old, new, 1))

path = Path("src/ertmp/multitrack_media.rs")
text = path.read_text()
old = '''            b'h', b'v', b'c', b'1', 1, 0, 0, 0, 1, 0xBB,
'''
new = '''            b'h', b'v', b'c', b'1', 1, 0, 0, 1, 0xBB,
'''
if text.count(old) != 1:
    raise RuntimeError(f"expected one ManyTracksManyCodecs fixture, found {text.count(old)}")
path.write_text(text.replace(old, new, 1))

print("Applied wildcard and multicodec follow-up")
