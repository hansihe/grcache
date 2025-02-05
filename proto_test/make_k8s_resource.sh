#!/bin/bash
set -xe

IMAGE_B64=$(cat image.binpb | base64)

cat > grcache-service-test.yaml << EOL
apiVersion: hansihe.com/v1
kind: GrcacheService
metadata:
  name: service-test
spec:
  descriptorSetSource:
    inline:
      data: ${IMAGE_B64}
  serviceName: example.ExampleService
  upstream:
    dns:
      url: foo.bar.baz
EOL
