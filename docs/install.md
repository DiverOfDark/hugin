# Install

```bash
helm install hugin oci://ghcr.io/diverofdark/charts/hugin \
  --namespace hugin --create-namespace \
  --set ingress.enabled=true \
  --set 'ingress.hosts[0].host=status.example.com' \
  --set 'ingress.hosts[0].paths[0].path=/' \
  --set 'ingress.hosts[0].paths[0].pathType=Prefix'
```

The chart also lives in this repository under `charts/hugin`, which is what an Argo CD Application can point at.

The dashboard needs [metrics-server](https://github.com/kubernetes-sigs/metrics-server) for CPU and memory figures. Without it the status dots still work.
