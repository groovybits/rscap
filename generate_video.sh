#!/bin/bash

ffmpeg -f lavfi -i smptebars=size=1280x720:rate=30 -f lavfi -i sine=frequency=1000:sample_rate=48000 \
       -vf "drawtext=fontfile=/Library/Fonts/Arial.ttf:timecode='00\:00\:00\:00':rate=30:text='TCR\: %{pts\:hms}':fontsize=48:fontcolor=white@0.8:x=10:y=10, \
            drawtext=fontfile=/Library/Fonts/Arial.ttf:text='Frame\: %{n}':fontsize=48:fontcolor=white@0.8:x=10:y=70, \
            drawtext=fontfile=/Library/Fonts/Arial.ttf:text='Size\: %{pict_w}x%{pict_h}':fontsize=48:fontcolor=white@0.8:x=10:y=130" \
       -c:v libx264 -c:a aac -b:a 128k -ar 48000 -ac 2 \
       -mpegts_pmt_start_pid 0x1000 -mpegts_start_pid 0x0100 \
       -metadata service_provider="TestStream" -metadata service_name="ColorBarsWithTone" \
       -f mpegts udp://224.0.0.200:10000?pkt_size=188&fifo_size=7

