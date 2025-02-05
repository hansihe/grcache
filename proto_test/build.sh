#!/bin/bash
set -xe

# buf build -o image.binpb
protoc -I ../proto -I . dummy_service.proto ../proto/google/protobuf/descriptor.proto ../proto/grcache/options.proto -oimage.binpb --retain_options
