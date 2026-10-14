#!/bin/sh
# A 4 second test video with sound for the Video Player tests.
mkdir -p "$HOME/Videos"
ffmpeg -hide_banner -loglevel error -y \
	-f lavfi -i "testsrc2=size=640x360:rate=30:duration=4" \
	-f lavfi -i "sine=frequency=440:duration=4" \
	-c:v libx264 -pix_fmt yuv420p -c:a aac -shortest "$HOME/Videos/obstools-test.mp4"

# Two pictures for the Image Source tests.
mkdir -p "$HOME/Pictures"
magick -size 400x225 xc:'#2b7fff' -gravity center -pointsize 48 -fill white -annotate 0 'BRB' "$HOME/Pictures/obstools-brb.png"
magick -size 400x225 xc:'#30a46c' -gravity center -pointsize 48 -fill white -annotate 0 'GG' "$HOME/Pictures/obstools-gg.png"
