#!/bin/sh
# export.sh piece
p=$1
easel export $p.easel -o $p@1x.png --scale 1 >/dev/null
easel export $p.easel -o $p@2x.png --scale 2 >/dev/null
easel export $p.easel -o $p@1x-dark.png --show preview-bg >/dev/null
easel export $p.easel -o $p@4x-dark.png --show preview-bg --scale 4 >/dev/null
echo exported $p
