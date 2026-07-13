# Warm Pool Quickstart: See the 6x Speedup in 5 Minutes

**What you'll see**: sandbox creation drops from ~19s to ~2.7s
with pre-provisioned warm pools.

## Prerequisites

- `oc` CLI ([download](https://mirror.openshift.com/pub/openshift-v4/clients/ocp/latest/))
- `openshell` CLI ([download](https://github.com/NVIDIA/OpenShell/releases))

## 1. Connect (30 seconds)

```shell
# Login to the cluster
oc login -u admin -p '<CLUSTER_PASSWORD>' \
  https://api.warm-pool-rerun.hkz1.p3.openshiftapps.com:443 \
  --insecure-skip-tls-verify

# Register the gateway (order matters: add first, then install certs)
openshell gateway add \
  --name warm-pool-demo \
  --local \
  https://openshell-openshell.apps.rosa.warm-pool-rerun.hkz1.p3.openshiftapps.com

# Install mTLS certificates AFTER gateway add (it overwrites auto-generated ones)
tar xzf warm-pool-demo-certs.tar.gz -C ~/.config/openshell/

openshell gateway select warm-pool-demo
```

The `warm-pool-demo-certs.tar.gz` file is included alongside this guide.

<details>
<summary>Alternative: extract certs from the cluster yourself</summary>

If you don't have the tarball, run `gateway add` first, then overwrite the
auto-generated certs:

```shell
openshell gateway add --name warm-pool-demo --local \
  https://openshell-openshell.apps.rosa.warm-pool-rerun.hkz1.p3.openshiftapps.com

GATEWAY_DIR=~/.config/openshell/gateways/warm-pool-demo/mtls
kubectl -n openshell get secret openshell-client-tls \
  -o jsonpath='{.data.ca\.crt}' | base64 -d > "$GATEWAY_DIR/ca.crt"
kubectl -n openshell get secret openshell-client-tls \
  -o jsonpath='{.data.tls\.crt}' | base64 -d > "$GATEWAY_DIR/tls.crt"
kubectl -n openshell get secret openshell-client-tls \
  -o jsonpath='{.data.tls\.key}' | base64 -d > "$GATEWAY_DIR/tls.key"
```

</details>

## 2. Run both, compare (60 seconds)

The warm pool is pre-configured for the `base` image (`:latest` tag).
Requesting a different tag bypasses the pool and goes through cold start.
Both commands use the same gateway, same cluster, same sandbox image
(just different tags of the same content).

```shell
# Warm pool path: image tag matches the pool
time openshell sandbox create --name warm-$(date +%s) --from base -- echo "warm pool"

# Cold start path: different tag, no matching pool
time openshell sandbox create --name cold-$(date +%s) \
  --from ghcr.io/nvidia/openshell-community/sandboxes/base:21aa171 \
  -- echo "cold start"
```

Expected output:

```
warm pool          ~2.7 seconds
cold start        ~14-19 seconds
```

That's it. The warm pool sandbox and the cold start sandbox are
functionally identical (same base image, same shell, same capabilities).
The only difference is startup time.

## 3. Try interactive SSH (optional, 30 seconds)

```shell
# Drop into a shell inside a warm pool sandbox
openshell sandbox create --name ssh-demo-$(date +%s) --from base

# You're now in bash inside the sandbox. Try:
#   ls, whoami, cat /etc/hostname, exit
```

## How it works

The gateway checks if a warm pool exists for the requested image.
If it finds one with ready pods, it claims a pre-provisioned pod
and pushes identity via gRPC (~500ms). If no pool matches, it
falls back to cold start (new pod from scratch, ~16s).

```
Cold:  CLI -> Gateway -> create Pod -> pull image -> start supervisor -> ready  (~16s)
Warm:  CLI -> Gateway -> claim warm pod -> push identity via gRPC -> ready     (~2.7s)
```

Warm pool pods run an "unidentified" supervisor: no gateway connection,
no identity, no policies. They become real sandboxes only when the
gateway pushes credentials after claiming. This means policies are
always fresh (compiled at claim time, not pool time).

## Troubleshooting

**"invalid peer certificate: BadSignature"**: The mTLS certs are wrong.
Re-extract the tarball AFTER `gateway add` (it overwrites auto-generated
certs): `tar xzf warm-pool-demo-certs.tar.gz -C ~/.config/openshell/`

**"error: You must be logged in"**: Run the `oc login` command from step 1.

**"No gateway selected"**: Run `openshell gateway select warm-pool-demo`.

**Both runs take the same time (~16s)**: The warm pool may have been
drained by other testers. Check pool status:
`kubectl -n openshell get sandboxwarmpool openshell-grpc-pool`
(should show READY=3). Pools auto-replenish after each claim.
