# 產生 Office／OpenDocument 測試檔（給 src/office.rs 的測試用）：python3 testdata/make_office.py
import zipfile, os
here = os.path.join(os.path.dirname(os.path.abspath(__file__)), "office")
os.makedirs(here, exist_ok=True)

def write(name, files, stored_first=None):
    with zipfile.ZipFile(os.path.join(here, name), "w", zipfile.ZIP_DEFLATED) as z:
        if stored_first:
            z.writestr(zipfile.ZipInfo(stored_first[0]), stored_first[1], compress_type=zipfile.ZIP_STORED)
        for n, data in files.items():
            z.writestr(n, data)

CT = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>'
def rels(*items):
    body = "".join(f'<Relationship Id="{i}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{t}" Target="{target}"/>' for i, t, target in items)
    return f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{body}</Relationships>'

W = 'xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"'
def wp(text, ppr=""):
    return f'<w:p>{"<w:pPr>"+ppr+"</w:pPr>" if ppr else ""}<w:r><w:t xml:space="preserve">{text}</w:t></w:r></w:p>'
doc = f'''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document {W}><w:body>
{wp("專案報告", '<w:pStyle w:val="1"/>')}
<w:p><w:pPr><w:tabs><w:tab w:val="left" w:pos="720"/></w:tabs></w:pPr><w:r><w:t>第一段：A &amp; B</w:t></w:r><w:r><w:tab/><w:t>有定位點</w:t></w:r><w:del><w:r><w:delText>刪除的文字</w:delText></w:r></w:del><w:r><w:instrText> PAGE </w:instrText></w:r></w:p>
{wp("第二章", '<w:pStyle w:val="2"/>')}
{wp("項目一", '<w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr>')}
{wp("子項目", '<w:numPr><w:ilvl w:val="1"/><w:numId w:val="1"/></w:numPr>')}
{wp("取消編號的段落", '<w:numPr><w:ilvl w:val="0"/><w:numId w:val="0"/></w:numPr>')}
<w:p/>
{wp("   ")}
<w:tbl><w:tblPr/><w:tr><w:tc>{wp("名稱")}</w:tc><w:tc>{wp("數量")}</w:tc></w:tr>
<w:tr><w:tc>{wp("蘋果")}</w:tc><w:tc>{wp("3")}</w:tc></w:tr>
<w:tr><w:tc>{wp("香蕉")}{wp("（熟）")}</w:tc><w:tc>{wp("12")}</w:tc></w:tr></w:tbl>
<w:p><w:r><w:t>換行前</w:t><w:br/><w:t>換行後</w:t></w:r></w:p>
{wp("Heading3 內建樣式", '<w:pStyle w:val="Heading3"/>')}
{wp("樣式清單第二層", '<w:pStyle w:val="ListBullet2"/>')}
{wp("樣式清單但取消編號", '<w:pStyle w:val="ListBullet2"/><w:numPr><w:numId w:val="0"/></w:numPr>')}
<w:sectPr/></w:body></w:document>'''
styles = f'''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles {W}><w:style w:type="paragraph" w:default="1" w:styleId="a"><w:name w:val="Normal"/></w:style>
<w:style w:type="paragraph" w:styleId="1"><w:name w:val="heading 1"/><w:basedOn w:val="a"/><w:pPr><w:outlineLvl w:val="0"/></w:pPr></w:style>
<w:style w:type="paragraph" w:styleId="2"><w:name w:val="heading 2"/><w:pPr><w:outlineLvl w:val="1"/></w:pPr></w:style>
<w:style w:type="paragraph" w:styleId="ListBullet2"><w:name w:val="List Bullet 2"/><w:pPr><w:numPr><w:numId w:val="2"/></w:numPr></w:pPr></w:style></w:styles>'''
write("sample.docx", {
    "_rels/.rels": rels(("rId1", "officeDocument", "word/document.xml")),
    "word/document.xml": doc,
    "word/styles.xml": styles,
}, stored_first=("[Content_Types].xml", CT))

P = 'xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"'
def sp(ph, paras):
    nv = f'<p:nvSpPr><p:cNvPr id="2" name="x"/><p:cNvSpPr/><p:nvPr>{ph}</p:nvPr></p:nvSpPr>'
    body = "".join(f'<a:p>{f'<a:pPr lvl="{l}"/>' if l else ""}<a:r><a:rPr lang="zh-TW"/><a:t>{t}</a:t></a:r></a:p>' for l, t in paras)
    return f'<p:sp>{nv}<p:spPr/><p:txBody><a:bodyPr/>{body}</p:txBody></p:sp>'
slide_open = f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><p:sld {P}><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/>'
slide_close = '</p:spTree></p:cSld></p:sld>'
s_open = slide_open + sp('<p:ph type="ctrTitle"/>', [(0, "開場")]) + sp('<p:ph type="subTitle" idx="1"/>', [(0, "副標題文字")]) + sp('<p:ph type="dt" idx="10"/>', [(0, "2026/11/22")]) + sp('<p:ph type="sldNum" idx="12"/>', [(0, "1")]) + slide_close
table = '<p:graphicFrame><p:nvGraphicFramePr/><a:graphic><a:graphicData><a:tbl><a:tr h="1"><a:tc><a:txBody><a:p><a:r><a:t>季</a:t></a:r></a:p></a:txBody></a:tc><a:tc><a:txBody><a:p><a:r><a:t>營收</a:t></a:r></a:p></a:txBody></a:tc></a:tr><a:tr h="1"><a:tc><a:txBody><a:p><a:r><a:t>Q1</a:t></a:r></a:p></a:txBody></a:tc><a:tc><a:txBody><a:p><a:r><a:t>100</a:t></a:r></a:p></a:txBody></a:tc></a:tr></a:tbl></a:graphicData></a:graphic></p:graphicFrame>'
s_main = slide_open + sp('<p:ph type="title"/>', [(0, "重點")]) + sp('<p:ph idx="1"/>', [(0, "第一點"), (1, "細節")]) + table + sp('', [(0, "文字方塊")]) + slide_close
pres = f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><p:presentation {P}><p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst><p:sldIdLst><p:sldId id="256" r:id="rId2"/><p:sldId id="257" r:id="rId3"/></p:sldIdLst></p:presentation>'
write("sample.pptx", {
    "[Content_Types].xml": CT,
    "_rels/.rels": rels(("rId1", "officeDocument", "ppt/presentation.xml")),
    "ppt/presentation.xml": pres,
    "ppt/_rels/presentation.xml.rels": rels(("rId1", "slideMaster", "slideMasters/slideMaster1.xml"), ("rId2", "slide", "slides/slide2.xml"), ("rId3", "slide", "slides/slide1.xml")),
    "ppt/slides/slide1.xml": s_main,
    "ppt/slides/slide2.xml": s_open,
    "docProps/thumbnail.jpeg": b"\xff\xd8\xff\xd9",
})

X = 'xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"'
wb = f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook {X}><workbookPr defaultThemeVersion="1"/><sheets><sheet name="隱藏" sheetId="3" state="hidden" r:id="rId1"/><sheet name="銷售" sheetId="1" r:id="rId2"/><sheet name="說明" sheetId="2" r:id="rId3"/></sheets></workbook>'
sst = f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><sst {X} count="9" uniqueCount="9">' + "".join(f"<si><t>{t}</t></si>" for t in ["日期", "品項", "金額", "比例", "已付", "備註"]) + '<si><r><t>蘋</t></r><r><rPr><b/></rPr><t>果</t></r><rPh sb="0" eb="1"><t>ピン</t></rPh></si><si><t>香蕉</t></si><si><t xml:space="preserve">芭樂</t></si></sst>'
sty = f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><styleSheet {X}><numFmts count="1"><numFmt numFmtId="164" formatCode="yyyy/mm/dd\\ hh:mm"/></numFmts><cellXfs count="4"><xf numFmtId="0"/><xf numFmtId="14"/><xf numFmtId="164"/><xf numFmtId="10"/></cellXfs></styleSheet>'
def c(ref, v, t=None, s=None):
    attrs = f' r="{ref}"' + (f' t="{t}"' if t else "") + (f' s="{s}"' if s else "")
    return f"<c{attrs}><v>{v}</v></c>"
sheet2 = f'''<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet {X}><dimension ref="A1:F5"/><sheetData>
<row r="1">{"".join(c(col + "1", i, "s") for i, col in enumerate("ABCDEF"))}</row>
<row r="2">{c("A2", 44927, s=1)}{c("B2", 6, "s")}{c("C2", 1234.5)}{c("D2", 0.125, s=3)}{c("E2", 1, "b")}<c r="F2" t="inlineStr"><is><t>直接輸入</t></is></c></row>
<row r="3">{c("A3", "45351.572916666664", s=2)}{c("B3", 7, "s")}{c("C3", "0.30000000000000004")}<c r="D3" s="3"/>{c("E3", 0, "b")}</row>
<row r="5"><c r="B5" t="s"><v>8</v></c><c r="C5"><f>SUM(C2:C3)*1000</f><v>1000000</v></c></row>
</sheetData></worksheet>'''
write("sample.xlsx", {
    "[Content_Types].xml": CT,
    "_rels/.rels": rels(("rId1", "officeDocument", "xl/workbook.xml")),
    "xl/workbook.xml": wb,
    "xl/_rels/workbook.xml.rels": rels(("rId1", "worksheet", "worksheets/sheet1.xml"), ("rId2", "worksheet", "/xl/worksheets/sheet2.xml"), ("rId3", "worksheet", "worksheets/sheet3.xml"), ("rId4", "sharedStrings", "sharedStrings.xml"), ("rId5", "styles", "styles.xml")),
    "xl/sharedStrings.xml": sst,
    "xl/styles.xml": sty,
    "xl/worksheets/sheet1.xml": f'<worksheet {X}><sheetData><row r="1"><c r="A1"><v>999</v></c></row></sheetData></worksheet>',
    "xl/worksheets/sheet2.xml": sheet2,
    "xl/worksheets/sheet3.xml": f'<worksheet {X}><sheetData/></worksheet>',
})

O = 'xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:presentation="urn:oasis:names:tc:opendocument:xmlns:presentation:1.0" office:version="1.3"'
def odf(name, mime, body):
    write(name, {"META-INF/manifest.xml": "<manifest/>", "content.xml": f'<?xml version="1.0" encoding="UTF-8"?><office:document-content {O}><office:body>{body}</office:body></office:document-content>'}, stored_first=("mimetype", mime))
odf("sample.odt", "application/vnd.oasis.opendocument.text", '''<office:automatic-styles><style:style xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" style:name="P1" style:family="paragraph" style:parent-style-name="Title"/></office:automatic-styles><office:text><text:p text:style-name="P1">文件標題</text:p><text:sequence-decls><text:sequence-decl text:name="Table"/></text:sequence-decls>
<text:tracked-changes><text:changed-region><text:deletion><text:p>刪掉的</text:p></text:deletion></text:changed-region></text:tracked-changes>
<text:h text:outline-level="1">標題</text:h>
<text:p>內文<text:s text:c="2"/>兩個<office:annotation><text:p>註解不顯示</text:p></office:annotation>空格</text:p>
<text:list><text:list-item><text:p>清單一</text:p><text:list><text:list-item><text:p>巢狀清單</text:p></text:list-item></text:list></text:list-item></text:list>
<table:table table:name="表格1"><table:table-column/><table:table-row><table:table-cell><text:p>A</text:p></table:table-cell><table:table-cell><text:p>B</text:p></table:table-cell></table:table-row><table:table-row><table:table-cell><text:p>1</text:p></table:table-cell><table:table-cell><text:p>2</text:p></table:table-cell></table:table-row></table:table>
<text:p/></office:text>''')
odf("sample.odp", "application/vnd.oasis.opendocument.presentation", '''<office:presentation>
<draw:page draw:name="page1"><draw:frame presentation:class="title"><draw:text-box><text:p>簡報標題</text:p></draw:text-box></draw:frame>
<draw:frame presentation:class="outline"><draw:text-box><text:list><text:list-item><text:p>大綱一</text:p></text:list-item></text:list></draw:text-box></draw:frame>
<draw:frame><table:table><table:table-row><table:table-cell><text:p>甲</text:p></table:table-cell><table:table-cell><text:p>乙</text:p></table:table-cell></table:table-row></table:table></draw:frame>
<draw:frame presentation:class="page-number"><draw:text-box><text:p>1</text:p></draw:text-box></draw:frame>
<presentation:notes><draw:frame><draw:text-box><text:p>備忘稿不顯示</text:p></draw:text-box></draw:frame></presentation:notes></draw:page>
<draw:page draw:name="page2"><draw:custom-shape/></draw:page></office:presentation>''')
odf("sample.ods", "application/vnd.oasis.opendocument.spreadsheet", '''<office:spreadsheet><table:table table:name="工作表一"><table:table-column table:number-columns-repeated="3"/>
<table:table-row><table:table-cell><text:p>名稱</text:p></table:table-cell><table:table-cell/><table:table-cell><text:p>值</text:p></table:table-cell></table:table-row>
<table:table-row table:number-rows-repeated="2"><table:table-cell><text:p>x</text:p></table:table-cell><table:table-cell/><table:table-cell office:value-type="float" office:value="1"><text:p>1</text:p></table:table-cell></table:table-row>
<table:table-row><table:table-cell table:number-columns-repeated="2"/><table:table-cell><text:p>末</text:p></table:table-cell></table:table-row>
<table:table-row table:number-rows-repeated="1048570"><table:table-cell table:number-columns-repeated="1024"/></table:table-row>
</table:table><table:table table:name="工作表二"><table:table-row><table:table-cell><text:p>不顯示</text:p></table:table-cell></table:table-row></table:table></office:spreadsheet>''')
print("ok")
