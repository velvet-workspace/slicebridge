"""Deterministic synthetic editable 3MF geometry corpus; not a real slicer acceptance fixture."""
from __future__ import annotations
import argparse
import hashlib
from pathlib import Path
import zipfile

HEAD = (b"<?xml version='1.0' encoding='UTF-8'?>\n"
        b"<model xmlns='http://schemas.microsoft.com/3dmanufacturing/core/2015/02' unit='millimeter'>\n"
        b"<resources><object id='1' type='model'><mesh><vertices>\n")
TAIL_VERTICES = b"</vertices><triangles>\n"
TAIL = b"</triangles></mesh></object></resources><build><item objectid='1'/></build></model>\n"

def emit_geometry(stream, tetrahedra: int) -> None:
    stream.write(HEAD)
    chunk = bytearray()
    for i in range(tetrahedra):
        x, y, z = (i % 100) * 0.5, ((i // 100) % 100) * 0.5, (i // 10000) * 0.5
        for dx, dy, dz in ((0, 0, 0), (0.2, 0, 0), (0, 0.2, 0), (0, 0, 0.2)):
            chunk.extend(f"<vertex x='{x+dx:.3f}' y='{y+dy:.3f}' z='{z+dz:.3f}'/>\n".encode())
        if len(chunk) >= 512 * 1024:
            stream.write(chunk)
            chunk.clear()
    if chunk:
        stream.write(chunk)
    stream.write(TAIL_VERTICES)
    chunk.clear()
    for i in range(tetrahedra):
        a = i * 4
        for u, v, w, c in ((0, 1, 2, '2C'), (0, 1, 3, '4'), (0, 2, 3, '4'), (1, 2, 3, '4')):
            chunk.extend(f"<triangle v1='{a+u}' v2='{a+v}' v3='{a+w}' paint_color='{c}'/>\n".encode())
        if len(chunk) >= 512 * 1024:
            stream.write(chunk)
            chunk.clear()
    if chunk:
        stream.write(chunk)
    stream.write(TAIL)

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path)
    parser.add_argument("tetrahedra", type=int)
    a = parser.parse_args()
    if not 1 <= a.tetrahedra <= 250000:
        parser.error("tetrahedra outside 1..250000")
    if a.destination.exists():
        parser.error("refusing to overwrite existing output")
    with zipfile.ZipFile(a.source) as source, zipfile.ZipFile(a.destination, "x", compression=zipfile.ZIP_DEFLATED, compresslevel=6) as target:
        for entry in source.infolist():
            if entry.filename == "3D/3dmodel.model":
                with target.open(entry.filename, "w", force_zip64=True) as stream:
                    emit_geometry(stream, a.tetrahedra)
            else:
                target.writestr(entry, source.read(entry.filename))
    with zipfile.ZipFile(a.destination) as check:
        model = check.getinfo("3D/3dmodel.model")
        print(f"file={a.destination} compressed_bytes={a.destination.stat().st_size} "
              f"model_xml_bytes={model.file_size} triangles={a.tetrahedra*4} "
              f"sha256={hashlib.sha256(a.destination.read_bytes()).hexdigest()}")

if __name__ == "__main__":
    main()
