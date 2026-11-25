# cmu-re-dev-screen-record

Developer tool for the CMU150: records the HMI screen (800x480) to H.264 at 24 fps.

The IPU converts the shown framebuffer page to I420 by DMA and the VPU encodes it
(`gst-launch-0.10 … vpuenc`), so the CPU only moves the frame bytes.

## Use

Install the package from the Store, then from the PC:

```sh
ssh -p 36000 cmu@<unit> /data_persist/cmu-re/pkg/cmu-re-dev-screen-record/bin/screen-record -t 20 > demo.h264
```

Or record to a file on the unit and copy it later (not affected by Wi-Fi):

```sh
ssh -p 36000 cmu@<unit> /data_persist/cmu-re/pkg/cmu-re-dev-screen-record/bin/screen-record -t 20 -o /tmp/demo.h264
```

```
screen-record [-t SECS] [-o FILE] [-b BITRATE] [-r FPS]
  default: stdout, until Ctrl-C / SIGTERM / ssh closed, 4000000 bit/s, 24 fps
```

At the end it prints `frames=N dropped=D fps=X` to stderr. With `-o` and no `-t`, stop it with
`ssh -t` + Ctrl-C or `kill` (a plain ssh disconnect does not reach it).

## Play

The output is a raw H.264 stream without timestamps:

```sh
ffplay -framerate 24 demo.h264
ffmpeg -framerate 24 -i demo.h264 -c copy demo.mp4
```

## Build

```sh
./t.sh cargo test
./t.sh cargo zigbuild --release --target arm-unknown-linux-musleabi
```

`t.sh` runs the `rvp-build` image (see `cmu-re-store/Containerfile`).
