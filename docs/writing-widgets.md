# Writing a widget by hand

A hand-written widget is a `Widget` resource with a Lua script:

```yaml
apiVersion: hugin.dev/v1alpha1
kind: Widget
metadata:
  name: trackhound
  namespace: trackhound
spec:
  script: |
    local orders = http.get("/orders"):check():json()
    return {
      { label = "Orders", value = fmt.number(#orders) },
    }
```

Select it on the Ingress with `hugin.dev/widget.type: trackhound`. The generator leaves a service alone once a hand-written `Widget` matches its type.

A `Widget` in hugin's namespace applies to every service with that `widget.type`, in any namespace.

The script API is described in [Script API](widget-api.md). A changed script is used at the next refresh, which is within seconds while the page is open.

## The sandbox

- A script can make HTTP requests only to the host in the service's `widget.url`.
- It never sees a credential. `secret("key")` returns a placeholder that hugin swaps in when the request is sent.
- It has no files, no operating system access and no way to load other code.
- Each run is limited to 12 requests, 32 MiB of memory and a fixed instruction budget.
