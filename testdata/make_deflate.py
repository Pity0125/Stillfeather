import zlib
def raw(data, level, strategy=zlib.Z_DEFAULT_STRATEGY):
    c = zlib.compressobj(level, zlib.DEFLATED, -15, 9, strategy)
    return c.compress(data) + c.flush()
text = "".join(f"Stillfeather 預覽測試 第 {i} 行：The quick brown fox jumps over the lazy dog.\n" for i in range(400)).encode()
x = 12345
rnd = bytearray()
for _ in range(5000):
    x = (x * 1103515245 + 12345) & 0x7FFFFFFF
    rnd.append((x >> 16) & 0xFF)
open("text-l9.deflate","wb").write(raw(text, 9))
open("text-l1.deflate","wb").write(raw(text, 1))
open("text-l0.deflate","wb").write(raw(text, 0))
open("random-l9.deflate","wb").write(raw(bytes(rnd), 9))
open("short-fixed.deflate","wb").write(raw(b"abcabcabcabcabc hello hello", 9, zlib.Z_FIXED))
open("empty.deflate","wb").write(raw(b"", 9))
