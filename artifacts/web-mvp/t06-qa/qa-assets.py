#!/usr/bin/env python3
"""QA 独立样例生成（不复用仓库任何生成器/校验器）。

产出：
  photo.png   512x512 真彩 PNG（zlib 压缩、CRC 自算）
  photo.jpg   由系统 sips 从 photo.png 转换（外部工具，独立于仓库代码）
  manual.pdf  3 页最小 PDF（手写 xref）
  big.pdf     40 MiB 有效 PDF（对象在前、注释填充，用于流式/内存观察）
  bomb.png    64 字节起、声明 60000x60000 的 PNG（CRC 正确，结构自洽）
  trunc.png   photo.png 去掉 IEND 尾部
  crc.png     photo.png 的 IDAT 数据翻转一个字节（CRC 失配）
  noeoi.jpg   photo.jpg 去掉末尾 EOI
  broken.pdf  有 %PDF- 头但无结构
  over-photo.bin  21 MiB 随机字节（照片上限是 20 MiB）
  over-request.bin 52 MiB 随机字节（路由体上限 = 51 MiB）
"""
import os
import struct
import sys
import zlib

out = sys.argv[1]
os.makedirs(out, exist_ok=True)


def png_chunk(kind: bytes, data: bytes) -> bytes:
    return (
        struct.pack(">I", len(data))
        + kind
        + data
        + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
    )


def make_png(width: int, height: int, fill=None) -> bytes:
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    raw = bytearray()
    for y in range(height):
        raw.append(0)  # filter: none
        for x in range(width):
            if fill is None:
                raw += bytes(((x * 7) % 256, (y * 5) % 256, ((x + y) * 3) % 256))
            else:
                raw += bytes(fill)
    idat = zlib.compress(bytes(raw), 9)
    return (
        b"\x89PNG\r\n\x1a\n"
        + png_chunk(b"IHDR", ihdr)
        + png_chunk(b"IDAT", idat)
        + png_chunk(b"IEND", b"")
    )


def make_pdf(pages: int, pad_bytes: int = 0) -> bytes:
    buf = bytearray(b"%PDF-1.4\n")
    offsets = [0]

    def add(body: bytes) -> None:
        offsets.append(len(buf))
        buf.extend(body)

    add(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n")
    kids = b" ".join(b"%d 0 R" % (3 + i) for i in range(pages))
    add(b"2 0 obj\n<< /Type /Pages /Count %d /Kids [%s] >>\nendobj\n" % (pages, kids))
    content = b"BT /F1 12 Tf 72 720 Td (QA page) Tj ET"
    contents_obj = 3 + pages
    font_obj = 3 + 2 * pages
    for i in range(pages):
        add(
            b"%d 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] "
            b"/Contents %d 0 R /Resources << /Font << /F1 %d 0 R >> >> >>\nendobj\n"
            % (3 + i, contents_obj + i, font_obj)
        )
    for i in range(pages):
        add(
            b"%d 0 obj\n<< /Length %d >>\nstream\n%s\nendstream\nendobj\n"
            % (contents_obj + i, len(content) + 1, content)
        )
    add(b"%d 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n" % font_obj)
    if pad_bytes:
        # 填充放在 xref 之前（PDF 阅读器与探针都要求 startxref 紧邻 EOF）；
        # 用注释行填充，不影响既有对象偏移。
        pad = pad_bytes - len(buf) - 64
        assert pad > 0
        buf.extend(b"%" + b"P" * pad + b"\n")
    xref_off = len(buf)
    count = len(offsets)  # 0..N
    buf.extend(b"xref\n0 %d\n0000000000 65535 f \n" % count)
    for offset in offsets[1:]:
        buf.extend(b"%010d 00000 n \n" % offset)
    buf.extend(b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n" % (count, xref_off))
    buf.extend(b"%%EOF\n")
    return bytes(buf)


def write(name: str, data: bytes) -> None:
    path = os.path.join(out, name)
    with open(path, "wb") as handle:
        handle.write(data)
    print(f"{name}\t{len(data)}\t{path}")


photo = make_png(512, 512)
write("photo.png", photo)

manual = make_pdf(3)
write("manual.pdf", manual)

big = make_pdf(3, pad_bytes=40 * 1024 * 1024)
write("big.pdf", big)

# 像素炸弹：IHDR 声明 60000x60000，IDAT 是占位字节（结构自洽、CRC 正确、文件很小）。
bomb = (
    b"\x89PNG\r\n\x1a\n"
    + png_chunk(b"IHDR", struct.pack(">IIBBBBB", 60000, 60000, 8, 2, 0, 0, 0))
    + png_chunk(b"IDAT", b"\x78\x01\x00\x00\x00\x01")
    + png_chunk(b"IEND", b"")
)
assert len(bomb) < 4096, len(bomb)
write("bomb.png", bomb)

write("trunc.png", photo[:-8])

corrupt = bytearray(photo)
corrupt[len(corrupt) - 20] ^= 0xFF  # IEND 之前的 IDAT 数据区
write("crc.png", bytes(corrupt))

write("noeoi.jpg", b"")  # 占位；稍后由 sips 转换并截断
write("broken.pdf", b"%PDF-1.4\n1 0 obj\ngarbage")

with open(os.path.join(out, "over-photo.bin"), "wb") as handle:
    handle.write(os.urandom(21 * 1024 * 1024))
with open(os.path.join(out, "over-request.bin"), "wb") as handle:
    handle.write(os.urandom(52 * 1024 * 1024))
print("over-photo.bin\t%d" % (21 * 1024 * 1024))
print("over-request.bin\t%d" % (52 * 1024 * 1024))
