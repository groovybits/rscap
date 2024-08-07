#!/bin/bash

ffmpeg -re -loglevel error -stats -y -hide_banner -f lavfi -i smptebars=size=1920x1080:rate=29.976 -f lavfi -i sine=frequency=1000:sample_rate=48000 \
       -c:v libx264 -c:a aac -b:a 128k -ar 48000 -ac 2 \
       -mpegts_pmt_start_pid 0x1000 -mpegts_start_pid 0x0100 \
       -metadata service_provider="TestStream" -metadata service_name="ColorBarsWithTone" \
       -nal-hrd cbr -maxrate 10M -minrate 10M -bufsize 10M -b:v 10M -muxrate 11M \
       -vf "drawtext=fontfile=/Library/Fonts/Arial.ttf:timecode='00\:00\:00\:00':rate=29.976:text='TCR\:%{pts\:hms}':fontsize=48:fontcolor=white@0.8:x=10:y=10, \
            drawtext=fontfile=/Library/Fonts/Arial.ttf:text='Frame\:%{n}':fontsize=48:fontcolor=white@0.8:x=10:y=70, \
            drawtext=fontfile=/Library/Fonts/Arial.ttf:text='Size\:%{pict_w}x%{pict_h}':fontsize=48:fontcolor=white@0.8:x=10:y=130" \
       -f mpegts "udp://224.0.0.200:10000?pkt_size=188&fifo_size=7&bitrate=10000000"

