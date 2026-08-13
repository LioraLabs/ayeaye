#!/bin/sh
set -eu

binary="$1"
output="$2"
cuda_home="${CUDA_HOME:-/usr/local/cuda}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/bundle/lib"
cp "$binary" "$work/bundle/ayeaye"
cp "$cuda_home/LICENSE" "$cuda_home/EULA.txt" "$work/bundle/"

ldd "$binary" | awk '$1 ~ /^(libcudart|libnvrtc|libcurand|libcublas|libcublasLt)\.so/ && $2 == "=>" { print $1, $3 }' |
while read -r soname path; do
  cp -L "$path" "$work/bundle/lib/$soname"
done
cp -a "$cuda_home"/lib64/libnvrtc-builtins.so.12* "$work/bundle/lib/"

test -f "$work/bundle/lib/libcudart.so.12"
test -f "$work/bundle/lib/libnvrtc.so.12"
patchelf --set-rpath '$ORIGIN/lib' "$work/bundle/ayeaye"
patchelf --print-rpath "$work/bundle/ayeaye" | grep -qx '\$ORIGIN/lib'
ldd "$work/bundle/ayeaye" >"$work/dependencies"
awk -v lib="$work/bundle/lib/" \
  '$1 ~ /^libcu/ && $1 != "libcuda.so.1" && index($3, lib) != 1 { exit 1 }' \
  "$work/dependencies"
awk '$1 != "libcuda.so.1" && /not found/ { exit 1 }' "$work/dependencies"
tar -czf "$output" -C "$work/bundle" .
