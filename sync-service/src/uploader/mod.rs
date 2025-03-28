mod ton;
mod tycho;

use async_trait::async_trait;

#[async_trait]
pub trait KeyBlockUploaderClient {
    async fn test(&self) -> anyhow::Result<()>;
}

pub struct KeyBlockUploader<T> {
    client: T,
}

impl<T: KeyBlockUploaderClient> KeyBlockUploader<T> {
    pub async fn new(client: T) -> anyhow::Result<Self> {
        Ok(Self { client })
    }
}
