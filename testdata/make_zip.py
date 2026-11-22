# 產生 ZIP 測試檔（給 src/zip.rs 的測試用）
import zipfile
with zipfile.ZipFile("sample.zip", "w") as z:
    z.writestr(zipfile.ZipInfo("hello.txt"), "hello stored", compress_type=zipfile.ZIP_STORED)
    z.writestr("dir/", "")
    z.writestr("dir/big.txt", "".join(f"line {i} 預覽\n" for i in range(2000)), compress_type=zipfile.ZIP_DEFLATED)
    z.writestr("中文名稱.xml", "<a>甲乙丙</a>", compress_type=zipfile.ZIP_DEFLATED)
