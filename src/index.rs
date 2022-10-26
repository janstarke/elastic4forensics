use anyhow::{bail, Result};
use base64::{encode_config, URL_SAFE_NO_PAD};
use elasticsearch::{
    BulkOperation, BulkParts, Elasticsearch,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::ecs::objects::ElasticObject;

struct ElasticDocument {
    id: String,
    content: Value,
}

impl From<ElasticDocument> for (String, Value) {
    fn from(me: ElasticDocument) -> Self {
        (me.id, me.content)
    }
}

impl From<Value> for ElasticDocument {
    fn from(val: Value) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(val.to_string());
        let result = hasher.finalize();
        Self {
            id: encode_config(result, URL_SAFE_NO_PAD),
            content: val,
        }
    }
}

pub struct Index {
    name: String,
    client: Elasticsearch,

    cache_size: usize,
    document_cache: Option<Vec<ElasticDocument>>,
}

impl Index {
    pub fn new(name: String, client: Elasticsearch) -> Self {
        Self {
            name,
            client,
            cache_size: 10000,
            document_cache: Some(Vec::new()),
        }
    }
    
    #[allow(dead_code)]
    pub async fn add_timeline_object<Obj: ElasticObject>(&mut self, object: Obj) -> Result<()> {
        for (_ts, doc) in object.documents() {
            self.add_bulk_document(doc).await?;
        }
        Ok(())
    }

    pub (crate) async fn add_bulk_document(&mut self, document: Value) -> Result<()> {
        if let Some(c) = self.document_cache.as_mut() {
            c.push(document.into())
        }

        if self.document_cache.as_ref().unwrap().len() >= self.cache_size {
            self.flush().await
        } else {
            Ok(())
        }
    }

    pub async fn flush(&mut self) -> Result<()> {

        match self.document_cache.as_ref() {
            None => log::trace!("There is no document cache"),

            Some(document_cache) => {
                log::info!("flushing document cache with {} entries", document_cache.len());
                if document_cache.is_empty() {
                    log::trace!("Document cache is empty");
                } else {
                    let parts = BulkParts::Index(&self.name);

                    let item_count = self.document_cache.as_ref().unwrap().len();
                    let items: Vec<BulkOperation<Value>> = self
                        .document_cache
                        .replace(Vec::new())
                        .unwrap()
                        .into_iter()
                        .map(|v| {
                            let (id, val) = v.into();
                            BulkOperation::create(id, val).into()
                        })
                        .collect();
                    let bulk = self.client.bulk(parts).body(items);

                    let response = bulk.send().await?;

                    if !response.status_code().is_success() {
                        log::error!(
                            "error {} while sending bulk operation",
                            response.status_code()
                        );
                        log::error!("{}", response.text().await?);
                        bail!("error while sending bulk operation");
                    } else {
                        let json: Value = response.json().await?;
                        if json["errors"].as_bool().unwrap() {
                            log::error!("error while writing to elasticsearch: {json}");
                        } else {
                            log::trace!("successfully wrote {item_count} items");
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn set_cache_size(&mut self, cache_size: usize) -> Result<()> {
        if self.cache_size > cache_size {
            self.flush().await?;
        }
        self.cache_size = cache_size;
        Ok(())
    }
}

impl Drop for Index {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}
