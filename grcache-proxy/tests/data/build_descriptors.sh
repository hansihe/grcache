#!/bin/bash
set -xe

buf build -o proto_descriptors.binpb --exclude-source-info
