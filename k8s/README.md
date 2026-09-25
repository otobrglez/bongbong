# Deploying the room server

The manifests for `bongbong-server` (docs/online-coop-prd.md §4.8). One
Deployment of one replica in namespace `bongbong-prod`, holding every
room in its memory; `kustomize` picks the image tag.

```
k8s/base/
  namespace.yaml          bongbong-prod
  deployment-rooms.yaml   ConfigMap, Deployment, Service, Ingress, ServiceMonitor
  kustomization.yaml      the resource list and the image tag
k8s/preview/
  rooms-preview.yaml      one PR's server, `__PR__` substituted by CI
```

## Per-PR previews

Every open PR gets its own room server in namespace `bongbong-pr-<N>`,
deployed by `.github/workflows/pr-server.yml` and reachable at
**`wss://rooms.bongbong.io/pr-<N>/ws`**.

The preview takes a *path* on production's hostname rather than a host of
its own, which is what makes it free: the one `rooms.bongbong.io` record
and Cloudflare's edge certificate already cover it, so a preview needs no
DNS record, no certificate and no new API token. nginx matches the
longest path first, so production keeps everything that is not
`/pr-<N>/...`, and the ingress rewrites `/pr-7/ws` to `/ws` before it
reaches the server. No client change is needed either - `--rooms
wss://rooms.bongbong.io/pr-7` already produces that socket URL, because
`net::rooms::socket_url` appends `/ws` to whatever base it is given.

The whole preview is one namespace, so teardown on PR close is a single
`kubectl delete namespace`. Previews run small - 100m of CPU and
`--max-rooms 3` - and drain in 30 s rather than production's half hour,
because a preview round is nobody's evening and `delete namespace` waits
out the grace period.

`rooms-preview.yaml` is a plain template with `__PR__` in it rather than
a kustomize overlay: every object is parameterised by the same one
number, which is the thing kustomize has no good answer for, and a `sed`
you can run by hand beats a patch you have to render to understand.

```
# what CI applies, rendered for PR 7
sed -e 's/__PR__/7/g' -e 's|__IMAGE__|...|g' k8s/preview/rooms-preview.yaml
```

Everything below runs over Tailscale: the registry and the cluster's API
server are both on the tailnet, so `tailscale status` has to be up first.

## Day to day

Deploys happen from CI: tagging `vX.Y.Z` runs
`.github/workflows/deploy-rooms.yml`, which builds and pushes the image
and rolls it out. That is the same tag `cloudflare-deploy.yml` uses, so
the web client and the server ship together - they refuse each other by
name if `net::PROTOCOL_VERSION` disagrees.

By hand, for a one-off:

```
just rooms-image        # build linux/amd64 locally
just rooms-image-run    # run that image on loopback, plain ws://
just rooms-push         # build, push, and point kustomization at the tag
just rooms-deploy       # kubectl apply -k, then wait the rollout out
just rooms-manifests    # what would be applied, without applying it
just rooms-logs         # follow the room server's logs
```

**A deploy with rounds in play is slow on purpose.** `Recreate` waits
for the old pod, and the old pod drains for as long as its longest round in
play - up to 30 minutes (`hub::DRAIN_MAX`). Nobody's round ends; new rooms
wait for the replacement. A room with no round in play (a lobby, a paused
round, an end screen) does not hold the drain: it closes at once, its players
told the server is restarting, so a deploy with nobody mid-round takes
seconds. To cut a drain short anyway:
`kubectl -n bongbong-prod delete pod <pod> --grace-period=0 --force`.

## One-time setup

Four things are not in these files, because they are the cluster's rather
than this repository's. **The registry pull secret is the one that blocks
a first rollout** - without it the pod sits in `ImagePullBackOff` with
"no basic auth credentials".

1. **Repository secrets**: `TS_OAUTH_CLIENT_ID`, `TS_OAUTH_SECRET`,
   `DOCKER_REGISTRY_USERNAME`, `DOCKER_REGISTRY_PASSWORD` and
   `KUBECONFIG`, the same names boo-run uses. `PUSHOVER_TOKEN` and
   `PUSHOVER_USER` are optional - the notify step skips itself when they
   are unset.

   **`KUBECONFIG` holds no credential.** The Tailscale operator's API
   server proxy authenticates by tailnet identity, so a kubeconfig for it
   is a server URL and the literal string `token: "unused"`:

   ```yaml
   apiVersion: v1
   kind: Config
   clusters:
     - name: tailscale-operator
       cluster: { server: https://tailscale-operator.folk-decibel.ts.net }
   users:
     - name: tailscale-auth
       user: { token: "unused" }
   contexts:
     - name: tailscale-operator
       context: { cluster: tailscale-operator, user: tailscale-auth }
   current-context: tailscale-operator
   ```

   `tailscale configure kubeconfig tailscale-operator` writes exactly
   that. Base64 it into the secret. The one credential in the whole
   pipeline is the OAuth client that lets a runner become a `tag:ci`
   node; everything else follows from being that node.

2. **Authorisation for `tag:ci`** - two halves that have to agree, and
   neither exists yet:

   *In the cluster*, `kubectl apply -f k8s/ci/deployer.yaml` creates a
   ClusterRole scoped to what the workflows do and binds it to the group
   `bongbong-deployers`.

   *On the tailnet* (Access controls -> JSON editor), a grant maps
   `tag:ci` onto that group. Add it as the first entry of `"grants"`:

   ```
   {"src": ["tag:ci"], "dst": ["tag:k8s-operator"],
    "app": {"tailscale.com/cap/kubernetes": [
      {"impersonate": {"groups": ["bongbong-deployers"]}}]}},
   ```

   Without it a `tag:ci` runner reaches the operator over the network -
   the existing wide-open grants already allow that - but arrives with no
   Kubernetes identity, and every `kubectl` call is refused.

3. **The registry pull secret, per namespace.** boo-run's Deployments
   declare no `imagePullSecrets` because `boo-prod` carries a `regcred`
   secret of type `kubernetes.io/dockerconfigjson` attached to the
   namespace's **default ServiceAccount**. That is a per-namespace thing,
   so `bongbong-prod` needs its own. Either copy the existing one:

   ```
   kubectl -n boo-prod get secret regcred -o json \
     | jq 'del(.metadata.namespace,.metadata.resourceVersion,.metadata.uid,
                .metadata.creationTimestamp,.metadata.managedFields,
                .metadata.ownerReferences)' \
     | kubectl -n bongbong-prod apply -f -
   ```

   or make a fresh one from the same credentials CI uses:

   ```
   kubectl create secret docker-registry regcred -n bongbong-prod \
     --docker-server=registry.folk-decibel.ts.net \
     --docker-username="$DOCKER_REGISTRY_USERNAME" \
     --docker-password="$DOCKER_REGISTRY_PASSWORD"
   ```

   Then attach it, which is what makes the Deployment need no mention of
   it:

   ```
   kubectl patch serviceaccount default -n bongbong-prod \
     -p '{"imagePullSecrets":[{"name":"regcred"}]}'
   kubectl -n bongbong-prod rollout restart deployment/rooms
   ```

4. **DNS - done.** `rooms.bongbong.io` is a **proxied CNAME to
   `ogrodje-one.boo.run`**, which is the shape every service on this node
   uses (`api.boo.run`, `authentication.boo.run`, `ci.boo.run` are the
   same). `ogrodje-one.boo.run` is the one A/AAAA record holding the
   node's real address (78.47.94.128), DNS-only, so the address lives in
   exactly one place and a CNAME to it follows a move for free. The
   cross-zone reference is the price of that; a local A/AAAA pair in
   bongbong.io would be self-contained but would have to be kept in step
   by hand.

   Proxied, not DNS-only, because Cloudflare terminates TLS at the edge
   and this cluster has no cert-manager. WebSockets pass the proxy, and
   its 100 s idle limit is never reached by a 20 Hz snapshot stream.

5. **TLS, and why there is none in these files.** This cluster runs no
   cert-manager at all - `clusterissuer` is not even a resource type - so
   the origin serves no certificate and Cloudflare terminates at the
   edge, exactly as it does for boo.run. §4.8 would rather have the
   socket DNS-only and off the proxy; that costs a cert-manager install
   and an issuer, and is worth doing only if the extra hop measures
   badly.

## Checking it

```
curl https://rooms.bongbong.io/health          # ok, or 503 while draining
curl https://rooms.bongbong.io/metrics | grep bongbong_
kubectl -n bongbong-prod get pods,svc,ingress
kubectl -n bongbong-prod rollout status deployment/rooms
```

Then play it: point a client at the deployed server with
`cargo run -- --rooms wss://rooms.bongbong.io` and press ONLINE, or open
`https://bongbong.io/?rooms=wss://rooms.bongbong.io`.

The two numbers worth an alert, both from `/metrics`:
`bongbong_tick_microseconds{quantile="0.99"}` nearing 16000, and
`bongbong_rooms` summing toward `--max-rooms`. Either is the signal to
build the several-instance design §4.8 defers, before players meet the
ceiling.
