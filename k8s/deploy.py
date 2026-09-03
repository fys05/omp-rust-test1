#!/usr/bin/env python3
"""Deploy to K8s via direct REST API. Direct JSON payloads (no YAML parsing — pitfall #42)."""
import base64
import json
import os
import ssl
import sys
import time
import urllib.request
import urllib.error

NS = os.environ["K8S_NAMESPACE"]
APP = os.environ["APP_NAME"]
DOMAIN = os.environ["DOMAIN"]
IMAGE = os.environ["IMAGE_TAG"]  # immutable digest: ghcr.io/...@sha256:...
PORT = int(os.environ["APP_PORT"])
HOST = os.environ["K8S_HOST"]
API_PORT = os.environ["K8S_PORT"]
API = f"https://{HOST}:{API_PORT}"
PG = f"{APP}-postgres"
PG_SECRET = f"{APP}-postgres-credentials"
DB_NAME = "library"

# --- TLS context: client cert auth, CA verified, SNI override for external IP ---
ca_path = "/tmp/k8s-ca.pem"
ctx = ssl.create_default_context(cafile=ca_path)
ctx.check_hostname = False  # IP not in SANs; CA chain still verified
cert_fd, cert_path = None, "/tmp/k8s-cert.pem"
key_path = "/tmp/k8s-key.pem"
ctx.load_cert_chain(certfile=cert_path, keyfile=key_path)


def api(method, path, body=None):
    req = urllib.request.Request(
        API + path,
        method=method,
        data=json.dumps(body).encode() if body is not None else None,
        headers={"Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(req, context=ctx, timeout=30) as r:
            raw = r.read().decode()
            return json.loads(raw) if raw else {}
    except urllib.error.HTTPError as e:
        if e.code == 404:
            return None
        detail = e.read().decode()[:500]
        print(f"API {method} {path} -> {e.code}: {detail}")
        raise


def create_or_update(kind, name, col_path, payload):
    res_path = f"{col_path}/{name}"  # pitfall #42a: never rsplit
    existing = api("GET", res_path)
    if existing:
        payload.setdefault("metadata", {})["resourceVersion"] = existing["metadata"]["resourceVersion"]
        api("PUT", res_path, payload)
        print(f"  UPDATED {kind}/{name}")
    else:
        api("POST", col_path, payload)
        print(f"  CREATED {kind}/{name}")


def b64(s):
    return base64.b64encode(s.encode()).decode()


def wait_rollout(kind_path, name, timeout=180):
    deadline = time.time() + timeout
    while time.time() < deadline:
        obj = api("GET", f"{kind_path}/{name}")
        if obj:
            spec_replicas = obj.get("spec", {}).get("replicas", 1)
            st = obj.get("status", {})
            ready = st.get("readyReplicas", 0)
            updated = st.get("updatedReplicas", st.get("updatedReplicas", 0))
            if ready >= spec_replicas and (kind_path.endswith("statefulsets") or st.get("observedGeneration", 0) >= obj["metadata"]["generation"]):
                print(f"  ROLLOUT OK {name} ({ready}/{spec_replicas} ready)")
                return True
        time.sleep(5)
    print(f"  ROLLOUT TIMEOUT {name}")
    return False


# --- Postgres credentials secret: merge missing keys on re-deploy (pitfall #40) ---
existing = api("GET", f"/api/v1/namespaces/{NS}/secrets/{PG_SECRET}")
db_url = f"postgres://postgres:PLACEHOLDER@{PG}.{NS}.svc.cluster.local:5432/{DB_NAME}"
if existing and "data" in existing:
    data = existing["data"]
    if "POSTGRES_PASSWORD" not in data:
        import secrets as pysecrets
        data["POSTGRES_PASSWORD"] = b64(pysecrets.token_urlsafe(24))
    pw = base64.b64decode(data["POSTGRES_PASSWORD"]).decode()
    if "POSTGRES_USER" not in data:
        data["POSTGRES_USER"] = b64("postgres")
    if "POSTGRES_DB" not in data:
        data["POSTGRES_DB"] = b64(DB_NAME)
    db_url = f"postgres://postgres:{pw}@{PG}.{NS}.svc.cluster.local:5432/{DB_NAME}"
    if data.get("DATABASE_URL") != b64(db_url):
        data["DATABASE_URL"] = b64(db_url)
    api("PUT", f"/api/v1/namespaces/{NS}/secrets/{PG_SECRET}", existing)
    print(f"  UPDATED secret/{PG_SECRET} (merged keys)")
else:
    import secrets as pysecrets
    pw = pysecrets.token_urlsafe(24)
    db_url = f"postgres://postgres:{pw}@{PG}.{NS}.svc.cluster.local:5432/{DB_NAME}"
    secret = {
        "apiVersion": "v1",
        "kind": "Secret",
        "metadata": {"name": PG_SECRET, "namespace": NS},
        "type": "Opaque",
        "data": {
            "POSTGRES_USER": b64("postgres"),
            "POSTGRES_PASSWORD": b64(pw),
            "POSTGRES_DB": b64(DB_NAME),
            "DATABASE_URL": b64(db_url),
        },
    }
    api("POST", f"/api/v1/namespaces/{NS}/secrets", secret)
    print(f"  CREATED secret/{PG_SECRET}")

# --- Postgres PVC ---
pvc = {
    "apiVersion": "v1",
    "kind": "PersistentVolumeClaim",
    "metadata": {"name": f"{PG}-data", "namespace": NS},
    "spec": {"accessModes": ["ReadWriteOnce"], "resources": {"requests": {"storage": "1Gi"}}},
}
pvc_path = f"/api/v1/namespaces/{NS}/persistentvolumeclaims"
if not api("GET", f"{pvc_path}/{PG}-data"):
    api("POST", pvc_path, pvc)
    print(f"  CREATED pvc/{PG}-data")
else:
    print(f"  EXISTS pvc/{PG}-data (kept)")

# --- Postgres StatefulSet ---
sts = {
    "apiVersion": "apps/v1",
    "kind": "StatefulSet",
    "metadata": {"name": PG, "namespace": NS, "labels": {"app": PG}},
    "spec": {
        "serviceName": PG,
        "replicas": 1,
        "selector": {"matchLabels": {"app": PG}},
        "template": {
            "metadata": {"labels": {"app": PG}},
            "spec": {
                "containers": [{
                    "name": "postgres",
                    "image": "postgres:16-alpine",
                    "ports": [{"containerPort": 5432}],
                    "envFrom": [{"secretRef": {"name": PG_SECRET}}],
                    "volumeMounts": [{"name": "data", "mountPath": "/var/lib/postgresql/data", "subPath": "pgdata"}],
                    "resources": {"requests": {"cpu": "50m", "memory": "128Mi"}, "limits": {"cpu": "500m", "memory": "512Mi"}},
                }],
                "volumes": [{"name": "data", "persistentVolumeClaim": {"claimName": f"{PG}-data"}}],
            },
        },
    },
}
create_or_update("StatefulSet", PG, f"/apis/apps/v1/namespaces/{NS}/statefulsets", sts)

create_or_update("Service", PG, f"/api/v1/namespaces/{NS}/services", {
    "apiVersion": "v1", "kind": "Service",
    "metadata": {"name": PG, "namespace": NS},
    "spec": {"type": "ClusterIP", "selector": {"app": PG},
             "ports": [{"name": "postgres", "port": 5432, "targetPort": 5432}]},
})

print("Waiting for postgres rollout...")
if not wait_rollout(f"/apis/apps/v1/namespaces/{NS}/statefulsets", PG, timeout=180):
    sys.exit("postgres rollout failed")

# --- App Deployment ---
deploy = {
    "apiVersion": "apps/v1",
    "kind": "Deployment",
    "metadata": {"name": APP, "namespace": NS, "labels": {"app": APP}},
    "spec": {
        "replicas": 1,
        "selector": {"matchLabels": {"app": APP}},
        "template": {
            "metadata": {"labels": {"app": APP}},
            "spec": {
                "containers": [{
                    "name": APP,
                    "image": IMAGE,
                    "ports": [{"containerPort": PORT}],
                    "envFrom": [{"secretRef": {"name": PG_SECRET}}],
                    "resources": {"requests": {"cpu": "50m", "memory": "64Mi"}, "limits": {"cpu": "500m", "memory": "256Mi"}},
                }],
            },
        },
    },
}
create_or_update("Deployment", APP, f"/apis/apps/v1/namespaces/{NS}/deployments", deploy)

create_or_update("Service", APP, f"/api/v1/namespaces/{NS}/services", {
    "apiVersion": "v1", "kind": "Service",
    "metadata": {"name": APP, "namespace": NS},
    "spec": {"type": "ClusterIP", "selector": {"app": APP},
             "ports": [{"name": "http", "port": PORT, "targetPort": PORT}]},
})

# --- Dual Ingress (cluster convention: no ingressClassName, traefik annotations) ---
TLS_SECRET = os.environ["TLS_SECRET"]
MIDDLEWARE = os.environ["MIDDLEWARE"]
ing_https = {
    "apiVersion": "networking.k8s.io/v1",
    "kind": "Ingress",
    "metadata": {"name": APP, "namespace": NS, "annotations": {
        "traefik.ingress.kubernetes.io/router.entrypoints": "websecure",
        "traefik.ingress.kubernetes.io/router.tls": "true",
    }},
    "spec": {
        "rules": [{"host": DOMAIN, "http": {"paths": [{
            "path": "/", "pathType": "Prefix",
            "backend": {"service": {"name": APP, "port": {"number": PORT}}},
        }]}}],
        "tls": [{"hosts": [DOMAIN], "secretName": TLS_SECRET}],
    },
}
create_or_update("Ingress", APP, f"/apis/networking.k8s.io/v1/namespaces/{NS}/ingresses", ing_https)

ing_http = {
    "apiVersion": "networking.k8s.io/v1",
    "kind": "Ingress",
    "metadata": {"name": f"{APP}-http", "namespace": NS, "annotations": {
        "traefik.ingress.kubernetes.io/router.entrypoints": "web",
        "traefik.ingress.kubernetes.io/router.middlewares": MIDDLEWARE,
    }},
    "spec": {
        "rules": [{"host": DOMAIN, "http": {"paths": [{
            "path": "/", "pathType": "Prefix",
            "backend": {"service": {"name": APP, "port": {"number": PORT}}},
        }]}}],
    },
}
create_or_update("Ingress", f"{APP}-http", f"/apis/networking.k8s.io/v1/namespaces/{NS}/ingresses", ing_http)

print("Waiting for app rollout...")
if not wait_rollout(f"/apis/apps/v1/namespaces/{NS}/deployments", APP, timeout=240):
    pods = api("GET", f"/api/v1/namespaces/{NS}/pods?labelSelector=app={APP}")
    for p in (pods or {}).get("items", []):
        print(f"  pod {p['metadata']['name']}: {p['status'].get('phase')}")
    sys.exit("app rollout failed")

print("DEPLOY COMPLETE")
