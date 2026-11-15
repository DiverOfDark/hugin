# The generator

Enable it in the chart:

```yaml
generator:
  enabled: true
  model: gpt-5-mini
  apiKeySecret:
    name: hugin-ai
    key: OPENAI_API_KEY
```

It talks to a Responses API (`/v1/responses`) and uses the provider's hosted web search tool. Set `generator.apiUrl` for another provider that implements both, or `generator.webSearch: false` for one without web search, such as a local model.

For each service with a `widget.type`, the generator:

1. Probes a few well-known paths of the service, such as `/openapi.json`, without credentials.
2. Starts the model as an agent, giving it the [script API](widget-api.md), the probe results and your hint. The model has three tools:
   - **web search**, to find the application's API documentation or source. Its first step must be a search, unless the service published an OpenAPI document.
   - **`run_script`**, which runs a Lua script against the service in the same sandbox the dashboard uses, and returns the result, every request made and the response sizes. The model explores the API with it.
   - **`submit_script`**, which hands in the final script.
3. Runs the submitted script against the service and checks the shape of what it returns: one to four blocks, short labels, no version numbers, not every value missing.
4. Has the card reviewed (see below). If the reviewer finds a problem, the model is told what it is and continues.
5. Stores the script if the card is also small enough to fetch every refresh. Otherwise the model is told why and continues.

With the script the model hands in a description of the application, a few words such as "Movie collection manager". It is stored as `spec.serviceDescription` of the `Widget` and shown as the second line of the card, unless the Ingress has a `description` annotation of its own.

## What the model is told to show

The model picks two to four values, in this order of preference:

1. What the application says is wrong with itself: a health state, failed items, alerts.
2. Work waiting or in progress: a queue, active downloads, unprocessed items.
3. The main thing the application holds: movies, documents, space used.
4. Current activity: a rate or a recent total.

It is told to leave out versions and settings, figures about the process itself (goroutines, heap, uptime), missing values and filler such as "API key present". How it decides between amber and red is described under [values that report a problem](widgets.md#values-that-report-a-problem).

## The review

A script that runs is not yet a good card: "335 MB/s" next to "0 active", "Uptime -", or Silenced alerts marked amber all pass the shape check. So a second model call judges every working card before it is stored. It starts without the history of the generation and gets:

- the same rules for what a card shows and how it marks problems that the generator was given,
- the service's name, application, image and your hint,
- the script, the blocks it returned with their status, and the requests it made with the start of each response.

It answers with a list of problems, each naming a block: values that contradict each other or the responses, missing values and filler, a status on a normal state, a number that cannot change, a field read from the wrong place. The problems go back to the generator, which fixes them or submits the same script again when the API has nothing better.

A card is sent back at most twice. The third working submission is kept without review. If the steps run out after a card was sent back, the last one sent back is kept. A review that fails or does not answer in the expected form accepts the card. So a wrong reviewer only makes the generation slower, it never leaves a service without a card. Reviews run only when a widget is written, not on refresh or re-checks. Turn them off with `generator.review: false`.

The model gets `generator.steps` replies per service. It can also end with a sentence saying why the card cannot be built, which becomes the `Widget`'s failure message.

The `Widget` is named `auto-<ingress name>` and lives in the service's namespace. It has an owner reference to the Ingress, so Argo CD lists it under the Ingress and it is deleted with it. Widgets for static services are owned by hugin's ConfigMap.

`kubectl get widgets -A` lists them with their status. `kubectl get widget <name> -o yaml` shows the script and what it returned when it was validated.

Things to know:

- **The model never sees credentials.** They are placeholders in the script and are redacted from response bodies. It does see the rest of what the service answers.
- **The model reads web pages, which are untrusted.** A page cannot reach your cluster: searches run at the provider, and the only thing the model can do here is run sandboxed scripts against the one service. A stored script can still call any endpoint of that service with its credential, so read a generated script once.
- **"Validated" means the script ran, returned values and passed the review.** The reviewer only sees what the API returned, so a plausible number that means something other than its label can still get through.
- **Documented applications work best.** For an application nobody has written about, the model relies on the probes, an OpenAPI document if the service has one, and your hint.
- **Versions and settings are refused.** A card is for what changes, so a script that returns a version number is sent back.
- **Use `widget.hint` when it gets something wrong**, for example to name the endpoint, an undocumented parameter, or say that no API key is set.
- **A script is written again** when the service's widget annotations change, on a major version bump of its image, when the `Widget` is deleted or annotated `hugin.dev/regenerate`, or when the stored script fails two re-checks in a row. Re-checks run every `generator.revalidateHours`.
- **A failed generation** leaves a `Widget` with `status.phase: Failed` and the reason. It is retried after ten minutes, then every `generator.retryHours`. A script that worked before is kept and stays in use.
- **A script that downloads a lot** is sent back three times for a lighter one, with a search of the documentation first, then kept if the model finds none.
- **Generated widgets no service asks for are deleted.**
- **The dashboard does not need the generator.** It runs as a separate Deployment and only writes `Widget` resources.

To try a generation without storing anything:

```bash
hugin draft <namespace>/<ingress> --ai-model gpt-5-mini --ai-reasoning-effort medium --hint "queue length and failures"
```

This prints the script. It needs `HUGIN_AI_KEY` and a kubeconfig; `--url` points it at a port-forward.
