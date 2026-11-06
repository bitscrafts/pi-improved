#!/usr/bin/env bash
# Rebuild _docs/manuals/pii-arch-dev.pdf from its source on lnx's texlive container (spec 389).
# The old PDF is MOVED to /projects/_trash first, never overwritten; the new one is verified (page count, and the
# new sentence is in the extracted text) before it is moved into place. Nothing else in the tree changes.
set -uo pipefail
cd /projects/pi-improved/_docs/manuals || exit 1
SRC=src/arch-dev; OUT=pii-arch-dev.pdf
RH=mvcorrea@10.10.10.217; RD='~/podman/latex-remote/pii-manuals/arch-dev'
[ -f "$SRC/src/main.tex" ] || { echo "REFUSED: $SRC/src/main.tex missing"; exit 2; }
ssh -o BatchMode=yes $RH "podman ps | grep -q latex-compiler-shared" || { echo "REFUSED: latex-compiler-shared is not running on lnx"; exit 3; }
W=$(mktemp -d /projects/_tmp/book-build.XXXXXX) || exit 1
echo "sync  : $SRC/src -> $RH:$RD/src"
ssh -o BatchMode=yes $RH "mkdir -p $RD/src $RD/build" || exit 4
tar -C "$SRC/src" -cf - . | ssh -o BatchMode=yes $RH "cd $RD/src && tar xf -" || exit 4
echo "compile: 4 passes (pdflatex, makeindex, biber, pdflatex x3)"
ssh -o BatchMode=yes $RH "podman exec latex-compiler-shared sh -c 'cd /workspace/pii-manuals/arch-dev/src && pdflatex -interaction=nonstopmode -output-directory=../build main.tex >../build/p1.log 2>&1; makeindex ../build/main.idx >/dev/null 2>&1; biber --output-directory ../build main >../build/biber.log 2>&1; pdflatex -interaction=nonstopmode -output-directory=../build main.tex >../build/p2.log 2>&1; pdflatex -interaction=nonstopmode -output-directory=../build main.tex >../build/p3.log 2>&1; pdflatex -interaction=nonstopmode -output-directory=../build main.tex >../build/p4.log 2>&1; echo done'"
scp -q $RH:$RD/build/main.pdf "$W/new.pdf" || { echo "FAILED: no PDF came back"; ssh $RH "tail -25 $RD/build/p4.log"; exit 5; }
ssh -o BatchMode=yes $RH "grep -c '^!' $RD/build/p4.log" | sed 's/^/latex errors in last pass: /'
uv run --no-project --with pymupdf python - "$W/new.pdf" "$OUT" <<'PY'
import sys, pymupdf
n = pymupdf.open(sys.argv[1]); o = pymupdf.open(sys.argv[2])
t = "".join(p.get_text() for p in n)
print("pages new/old:", n.page_count, o.page_count)
ok = "call_output.rs" in t.replace("\n", "")
print("new text names call_output.rs:", ok)
sys.exit(0 if (ok and n.page_count >= o.page_count - 2) else 1)
PY
RC=$?
if [ $RC -ne 0 ]; then echo "REFUSED: the rebuilt PDF failed verification; the old PDF is untouched ($W kept)"; exit 6; fi
mkdir -p /projects/_trash && mv "$OUT" "/projects/_trash/pii-arch-dev.pdf.old-$(date -u +%Y%m%dT%H%M%SZ)" && mv "$W/new.pdf" "$OUT"
echo "REPLACED: $OUT  $(stat -c %s $OUT) bytes"
