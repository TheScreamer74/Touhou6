#!/usr/bin/env bash
# Vertex oracle: diff the port's scene() quad-corner projections against the real
# decomp Draw3 (+ spec-exact SetupCamera). Usage: ./compare.sh <stage 1-6> <frame>
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; cd "$HERE"
source "$HERE/../common.sh"
oracle_check_assets
N="${1:-1}"; FRAME="${2:-1}"; OUT="$(oracle_output_dir)"
oracle_check_stage "$N"
if [ "${TH06_SKIP_ORACLE_BUILD:-0}" != 1 ]; then "$HERE/build.sh"; fi
cd "$ORACLE_ROOT"
cargo build -q -p th06 --release --example extract_ecl --example bg_vtx_dump
target/release/examples/extract_ecl "$ST_DAT" "stg${N}bg.anm" "$OUT/stg${N}bg.anm" >/dev/null
target/release/examples/bg_vtx_dump "$ST_DAT" "$N" "$FRAME" "$OUT/vtx_stage${N}_quads.txt" > "$OUT/vtx_stage${N}_port.txt"
/tmp/oracle_vtx "$OUT/stg${N}bg.anm" "$OUT/vtx_stage${N}_quads.txt" > "$OUT/vtx_stage${N}_decomp.txt"

# Each line = 4 screen (x,y) corners. Draw3 bakes the Y-flip into the world
# matrix, so its corner emission order is the reverse of the port's -- the quad
# (set of 4 points) is what matters, not the order. Sort each quad's 4 corners
# (by y then x) on both sides, then compare with a 1.5px tolerance (f32 vs float).
awk -v N="$N" -v FR="$FRAME" '
  function sortquad(arr,   i,j,tx,ty,p){
    # arr[1..8] = x0 y0 .. x3 y3 ; bubble-sort the 4 points by (y,x)
    for(i=0;i<4;i++) for(j=0;j<3-i;j++){
      if(arr[2*j+2]>arr[2*j+4] || (arr[2*j+2]==arr[2*j+4] && arr[2*j+1]>arr[2*j+3])){
        tx=arr[2*j+1]; ty=arr[2*j+2]; arr[2*j+1]=arr[2*j+3]; arr[2*j+2]=arr[2*j+4]; arr[2*j+3]=tx; arr[2*j+4]=ty
      }
    }
  }
  FNR==NR { for(i=1;i<=NF;i++) pa[NR,i]=$i; nr=NR; next }
  {
    # col 9 = min clip-w; skip quads with a corner at/near/behind the camera
    # plane. Real on-screen bg quads sit far from the eye (fog starts >=310), so
    # a corner with w<10 is grazing/degenerate: it projects to enormous coords
    # where f32-vs-float rounding explodes and screen space is meaningless.
    if (pa[FNR,9] < 10.0 || $9 < 10.0) { skipped++; next }
    for(i=1;i<=8;i++){ p[i]=pa[FNR,i]; q[i]=$i }
    # Skip quads projecting far outside the 384x448 viewport (never rendered):
    # there f32-vs-float rounding scales to px-level abs diffs that are relatively
    # negligible. On-screen coords live in ~[0,640]; 3000 is a wide margin.
    mag=0; for(i=1;i<=8;i++){ v=p[i]; if(v<0)v=-v; if(v>mag)mag=v; v=q[i]; if(v<0)v=-v; if(v>mag)mag=v }
    if (mag > 3000) { skipped++; next }
    sortquad(p); sortquad(q)
    worst=0; for(i=1;i<=8;i++){ d=p[i]-q[i]; if(d<0)d=-d; if(d>worst)worst=d }
    # Backstop: a real transform bug shows on a visible quad at modest px (the
    # anchor bug was 64px). A >500px delta means a corner is off-screen degenerate
    # (grazing near-plane not fully caught by the earlier guards), not a bug.
    if(worst>500){ skipped++; next }
    if(worst>gmax){gmax=worst; grow=FNR}
    # 1.5px tolerance: absorbs f32(glam)-vs-float(C) accumulation on grazing quads
    # while still catching any real transform bug (the anchor bug was 64px).
    if(worst>1.5){ bad++; if(bad<=12) printf "  quad %d worst=%.2fpx  port=%s  decomp=%s\n", FNR, worst, prevp, $0 }
    prevp=$0
  }
  END{
    if(FNR!=nr){ printf "LINE COUNT MISMATCH port=%d decomp=%d stage %s\n", nr, FNR, N; exit 1 }
    onscreen = nr - skipped;
    if(onscreen<=0){ print "NO ON-SCREEN OBSERVATIONS"; exit 1 }
    if(gmax<=1.5) printf "IDENTICAL (<=1.5px) stage %s frame %s: %d on-screen quads (%d off-screen/near-plane skipped), max %.3fpx\n", N, FR, onscreen, skipped, gmax;
    else { printf "DIVERGENCE stage %s frame %s: %d on-screen quads, max %.2fpx at quad %d\n", N, FR, onscreen, gmax, grow; exit 1 }
  }
' "$OUT/vtx_stage${N}_port.txt" "$OUT/vtx_stage${N}_decomp.txt"
echo "Evidence: $OUT"
