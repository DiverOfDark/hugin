//! Reads the Secrets that widgets reference. Access is by name only (`get`),
//! which is what lets the RBAC role list `resourceNames` instead of granting
//! list/watch on all Secrets.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use k8s_openapi::api::core::v1::Secret;
use kube::Api;

use crate::model::SecretRef;

const TTL_OK: Duration = Duration::from_secs(300);
const TTL_ERR: Duration = Duration::from_secs(60);

type Entry = (Instant, Result<BTreeMap<String, String>, String>);

pub struct SecretReader {
    client: Option<kube::Client>,
    cache: Mutex<HashMap<(String, String), Entry>>,
}

impl SecretReader {
    pub fn new(client: Option<kube::Client>) -> Self {
        Self {
            client,
            cache: Mutex::default(),
        }
    }

    /// Returns the value of one key, or a short reason it is unavailable.
    pub async fn get(&self, reference: &SecretRef) -> Result<String, String> {
        let cache_key = (reference.namespace.clone(), reference.name.clone());
        let cached = {
            let cache = self.cache.lock().unwrap();
            cache.get(&cache_key).and_then(|(at, value)| {
                let ttl = if value.is_ok() { TTL_OK } else { TTL_ERR };
                (at.elapsed() < ttl).then(|| value.clone())
            })
        };
        let data = match cached {
            Some(data) => data,
            None => {
                let fetched = self.fetch(reference).await;
                self.cache
                    .lock()
                    .unwrap()
                    .insert(cache_key, (Instant::now(), fetched.clone()));
                fetched
            }
        };
        data?.get(&reference.key).cloned().ok_or_else(|| {
            format!(
                "secret {}/{} has no key {}",
                reference.namespace, reference.name, reference.key
            )
        })
    }

    async fn fetch(&self, reference: &SecretRef) -> Result<BTreeMap<String, String>, String> {
        let Some(client) = &self.client else {
            return Err("no cluster access".into());
        };
        let api: Api<Secret> = Api::namespaced(client.clone(), &reference.namespace);
        let secret = api.get(&reference.name).await.map_err(|e| {
            let reason = match &e {
                kube::Error::Api(status) if status.code == 403 => "forbidden",
                kube::Error::Api(status) if status.code == 404 => "not found",
                _ => "unreadable",
            };
            tracing::warn!(
                namespace = %reference.namespace,
                secret = %reference.name,
                error = %e,
                "cannot read secret"
            );
            format!(
                "secret {}/{} is {reason}",
                reference.namespace, reference.name
            )
        })?;
        Ok(secret
            .data
            .unwrap_or_default()
            .into_iter()
            .map(|(key, value)| (key, String::from_utf8_lossy(&value.0).trim().to_string()))
            .collect())
    }
}
