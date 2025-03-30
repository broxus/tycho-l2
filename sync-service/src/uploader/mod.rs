mod ton;
mod tycho;

use async_trait::async_trait;

#[async_trait]
pub trait KeyBlockUploaderClient {
    async fn test(&self) -> anyhow::Result<()>;
}

#[async_trait]
impl KeyBlockUploaderClient for Box<dyn KeyBlockUploaderClient + Send + Sync> {
    async fn test(&self) -> anyhow::Result<()> {
        self.as_ref().test().await
    }
}

pub struct KeyBlockUploader<T> {
    client: T,
}

impl<T: KeyBlockUploaderClient> KeyBlockUploader<T> {
    pub async fn new(client: T) -> anyhow::Result<Self> {
        Ok(Self { client })
    }
}
