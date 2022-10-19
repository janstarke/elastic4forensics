use anyhow::{anyhow, Result};
use elasticsearch::{
    auth::Credentials,
    cat::CatIndicesParts,
    cert::CertificateValidation,
    http::{
        transport::{SingleNodeConnectionPool, TransportBuilder},
        Url,
    },
    indices::IndicesCreateParts,
    Elasticsearch,
};
use serde_json::{json, Value};

use crate::Index;

pub struct IndexBuilder {
    host: Option<String>,
    port: Option<u16>,
    index_name: String,
    do_certificate_validation: bool,
    credentials: Option<Credentials>,
}

const DEFAULT_HOST: &str = "localhost";
const DEFAULT_PORT: u16 = 9200;

pub trait WithHost<T> {
    fn with_host(self, host: T) -> Self;
}

impl IndexBuilder {
    pub fn with_name(index_name: String) -> Self {
        Self {
            host: None,
            port: None,
            index_name,
            do_certificate_validation: true,
            credentials: None,
        }
    }

    pub fn with_port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }

    pub fn without_certificate_validation(mut self) -> Self {
        self.do_certificate_validation = false;
        self
    }

    pub fn with_credentials(mut self, credentials: Credentials) -> Self {
        self.credentials = Some(credentials);
        self
    }

    pub fn host(&self) -> &str {
        match self.host.as_ref() {
            Some(h) => h,
            None => DEFAULT_HOST,
        }
    }

    pub fn port(&self) -> u16 {
        match self.port.as_ref() {
            Some(p) => *p,
            None => DEFAULT_PORT,
        }
    }

    pub fn index_name(&self) -> &str {
        &self.index_name
    }

    pub fn index_exists(&self) -> Result<bool> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;

        rt.block_on(self.do_index_exists())
    }

    pub async fn do_index_exists(&self) -> Result<bool> {
        let client = self.create_client()?;
        self.client_has_index(&client).await
    }

    pub fn build(&self) -> Result<Index> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;

        rt.block_on(self.do_build())
    }

    pub async fn do_build(&self) -> Result<Index> {
        let client = self.create_client()?;

        if !self.client_has_index(&client).await? {
            let index_body = json!({
                "mappings": {
                    "properties": {
                        "timestamp": {
                            "type": "date",
                            "format": "epoch_millis"
                        },
                    "file": {
                        "properties": {
                            "accessed": {
                                "type": "date",
                                "format": "epoch_millis"
                            },
                            "created": {
                                "type": "date",
                                "format": "epoch_millis"
                            },
                            "ctime": {
                                "type": "date",
                                "format": "epoch_millis"
                            },
                            "mtime": {
                                "type": "date",
                                "format": "epoch_millis"
                            },
                        }
                    }
                    }
                }
            });
            let parts = IndicesCreateParts::Index(&self.index_name);
            let response = client
                .indices()
                .create(parts)
                .body(index_body)
                .send()
                .await?;
            match response.error_for_status_code_ref() {
                Ok(_response) => (),
                Err(why) => {
                    log::error!(
                        "Error while creating index: {}",
                        response.text().await?
                    );
                    log::error!("error message was: {}", why);
                    return Err(anyhow!(why))
                }
            }

            //let pipeline_id = format!("{}_pipeline", self.index_name());
            //self.create_pipeline(&client, &pipeline_id).await?;
        }
        Ok(Index::new(self.index_name.clone(), client))
    }
/*
    async fn create_pipeline(&self, client: &Elasticsearch, pipeline_id: &str) -> Result<()> {
        let pipeline_parts = IngestPutPipelineParts::Id(pipeline_id);
        let set_timestamp = json!({
            "description": "Creates a timestamp when a document is initially indexed",
            "processors": [
                {
                    "set": {
                        "field": "timestamp",
                        "value": "{{{_ingest.timestamp}}}"
                    }
                }
            ]
        });
        let ingest_response = client
            .ingest()
            .put_pipeline(pipeline_parts)
            .body(set_timestamp)
            .send()
            .await?;

        match ingest_response.error_for_status_code_ref() {
            Err(why) => {
                log::error!(
                    "Error while creating pipeline: {}",
                    ingest_response.text().await?
                );
                log::error!("error message was: {}", why);
                Err(anyhow!(why))
            }
            Ok(_response) => {
                log::info!("sucessfully created pipeline {pipeline_id}");
                Ok(())
            }
        }
    }
*/
    fn create_client(&self) -> Result<Elasticsearch> {
        let url = Url::parse(&format!("http://{}:{}", self.host(), self.port()))?;
        let conn_pool = SingleNodeConnectionPool::new(url);
        let mut transport_builder = TransportBuilder::new(conn_pool)
            .cert_validation(if self.do_certificate_validation {
                CertificateValidation::Default
            } else {
                CertificateValidation::None
            })
            .disable_proxy();

        if let Some(credentials) = &self.credentials {
            transport_builder = transport_builder.auth(credentials.clone());
        }
        let transport = transport_builder.build()?;
        Ok(Elasticsearch::new(transport))
    }

    async fn client_has_index(&self, client: &Elasticsearch) -> Result<bool> {
        let response = client
            .cat()
            .indices(CatIndicesParts::Index(&["*"]))
            .format("json")
            .send()
            .await?;
        response.error_for_status_code_ref()?;

        if response.content_length().unwrap_or(0) == 0 {
            Ok(false)
        } else {
            let response_body = response.json::<Value>().await?;

            match response_body.as_array() {
                None => Ok(false),
                Some(body) => Ok(body
                    .iter()
                    .any(|r| *r["index"].as_str().unwrap() == self.index_name)),
            }
        }
    }
}

impl WithHost<String> for IndexBuilder {
    fn with_host(mut self, host: String) -> Self {
        self.host = Some(host);
        self
    }
}

impl WithHost<&str> for IndexBuilder {
    fn with_host(mut self, host: &str) -> Self {
        self.host = Some(host.to_owned());
        self
    }
}
