# Credentials from Secrets

Instead of writing an API key into an annotation, reference a Secret:

```yaml
metadata:
  annotations:
    hugin.dev/enabled: "true"
    hugin.dev/widget.type: argocd
    hugin.dev/widget.url: http://argocd-server.argocd.svc.cluster.local
    hugin.dev/widget.key-from-secret: argocd-readonly-token/token
```

`hugin.dev/widget.<field>-from-secret: <secret-name>/<key>` reads that key of that Secret and uses it as `widget.<field>`. The Secret must be in the same namespace as the Ingress.

Hugin may only read Secrets listed in the chart's `secretAccess` value:

```yaml
secretAccess:
  - namespace: argocd
    names: [argocd-readonly-token]
```

Each entry becomes a Role that allows `get` on exactly those Secrets. Hugin cannot list Secrets or read any other.

A plain `widget.<field>` annotation next to the reference is the fallback: it is used when the Secret cannot be read. This lets you add references first and delete the plaintext annotations afterwards. The metric `hugin_widget_credentials{source="annotation"}` counts widgets still using a plaintext credential; when it is zero the plaintext annotations can go.

Secrets are cached for five minutes, so a rotated key is picked up within that time.
