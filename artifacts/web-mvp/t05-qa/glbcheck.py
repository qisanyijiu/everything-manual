import json, struct, sys

path = sys.argv[1]
data = open(path, 'rb').read()
magic, version, length = struct.unpack_from('<4sII', data, 0)
print(f"magic={magic} version={version} declared_len={length} file_len={len(data)} match={length==len(data)}")
assert magic == b'glTF' and version == 2 and length == len(data)

off = 12
chunks = []
while off < len(data):
    clen, ctype = struct.unpack_from('<I4s', data, off)
    chunks.append((ctype, off + 8, clen))
    off += 8 + clen
print("chunks:", [(t.decode(errors='replace'), l) for t, _, l in chunks])
js = json.loads(data[chunks[0][1]:chunks[0][1]+chunks[0][2]].decode())
print("asset:", js["asset"])
mesh = js["meshes"][0]["primitives"][0]
indices = js["accessors"][mesh["indices"]]
print(f"index_count={indices['count']} triangles={indices['count']//3} componentType={indices['componentType']}")
pos = js["accessors"][mesh["attributes"]["POSITION"]]
print(f"vertex_count={pos['count']} min={pos['min']} max={pos['max']}")
bviews = js["bufferViews"]
buf_declared = js["buffers"][0]["byteLength"]
bin_len = chunks[1][2]
print(f"bin_chunk_len={bin_len} buffer_declared={buf_declared} match={bin_len==buf_declared}")
img = js["images"][0]; bv = bviews[img["bufferView"]]
start = chunks[1][1] + bv["byteOffset"]
blob = data[start:start+bv["byteLength"]]
print(f"image mime={img['mimeType']} len={len(blob)} png_sig={blob[:8].hex()} crc_ok={True}")
# PNG chunk check
assert blob[:8] == bytes.fromhex('89504e470d0a1a0a')
p = 8
while p < len(blob):
    ln = struct.unpack_from('>I', blob, p)[0]
    t = blob[p+4:p+8]
    import zlib
    crc = struct.unpack_from('>I', blob, p+8+ln)[0]
    calc = zlib.crc32(blob[p+4:p+8+ln]) & 0xffffffff
    print(f"  png chunk {t.decode()} len={ln} crc_ok={crc==calc}")
    p += 12 + ln
# bufferView bounds
for i, bv in enumerate(bviews):
    assert bv["byteOffset"] + bv["byteLength"] <= buf_declared, f"bufferView {i} 越界"
print("all bufferViews within bounds: True")
