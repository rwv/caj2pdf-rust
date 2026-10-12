// SPDX-License-Identifier: MIT
// Original public Win32 API logging control; no viewer dependencies.
#include <windows.h>
#include <stdio.h>
#include <string.h>
int main(void) {
    HDC dc = CreateCompatibleDC(NULL);
    if (!dc) return 1;
    BITMAPINFO bi;
    memset(&bi, 0, sizeof bi);
    bi.bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
    bi.bmiHeader.biWidth = 96;
    bi.bmiHeader.biHeight = -96;
    bi.bmiHeader.biPlanes = 1;
    bi.bmiHeader.biBitCount = 32;
    bi.bmiHeader.biCompression = BI_RGB;
    void *bits = NULL;
    HBITMAP bm = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &bits, NULL, 0);
    if (!bm || !bits) return 2;
    HGDIOBJ oldbm = SelectObject(dc, bm);
    HFONT font = CreateFontW(-24, 0, 0, 0, FW_NORMAL, 0, 0, 0,
        DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS,
        NONANTIALIASED_QUALITY, DEFAULT_PITCH, L"SimSun");
    if (!font) return 3;
    HGDIOBJ oldfont = SelectObject(dc, font);
    WCHAR face[64] = {0};
    WORD glyph = 0;
    GLYPHMETRICS gm;
    MAT2 mat = {{0,1},{0,0},{0,0},{0,1}};
    int facecount = GetTextFaceW(dc, 64, face);
    DWORD indices = GetGlyphIndicesW(dc, L"A", 1, &glyph, GGI_MARK_NONEXISTING_GLYPHS);
    DWORD outline = GetGlyphOutlineW(dc, glyph, GGO_BITMAP | GGO_GLYPH_INDEX,
        &gm, 0, NULL, &mat);
    DWORD data = GetFontData(dc, 0, 0, NULL, 0);
    BOOL text = ExtTextOutW(dc, 7, 11, 0, NULL, L"A", 1, NULL);
    DWORD source[4] = {0xff0000,0x00ff00,0x0000ff,0xffffff};
    BITMAPINFO small = bi;
    small.bmiHeader.biWidth = 2; small.bmiHeader.biHeight = -2;
    int stretch = StretchDIBits(dc, 31, 37, 13, 17, 0, 0, 2, 2,
        source, &small, DIB_RGB_COLORS, SRCCOPY);
    int set = SetDIBitsToDevice(dc, 67, 71, 2, 2, 0, 0, 0, 2,
        source, &small, DIB_RGB_COLORS);
    printf("face_count=%d indices=%lu glyph=%u outline_bytes=%lu font_bytes=%lu text=%d stretch=%d set=%d\n",
        facecount, indices, glyph, outline, data, text, stretch, set);
    SelectObject(dc, oldfont); SelectObject(dc, oldbm);
    DeleteObject(font); DeleteObject(bm); DeleteDC(dc);
    return !(facecount > 0 && indices != GDI_ERROR && glyph != 0xffff &&
        outline != GDI_ERROR && data != GDI_ERROR && text && stretch > 0 && set > 0);
}
