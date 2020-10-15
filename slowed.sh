ffmpeg -i pipe: -map 0 -c:v copy -filter:a aecho=1.0:0.7:20:0.5,asetrate=48000*0.85,aresample=48000*0.85,atempo=0.85 -b:a 48000 pipe:
