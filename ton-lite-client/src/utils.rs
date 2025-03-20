use anyhow::Result;

use crate::proto;

pub trait FromResponse: Sized {
    fn from_response(response: proto::Response) -> Result<Self>;
}

impl FromResponse for proto::Version {
    fn from_response(response: proto::Response) -> Result<Self> {
        match response {
            proto::Response::Version(item) => Ok(item),
            _ => anyhow::bail!("Unexpected TL message"),
        }
    }
}

impl FromResponse for proto::MasterchainInfo {
    fn from_response(response: proto::Response) -> Result<Self> {
        match response {
            proto::Response::MasterchainInfo(item) => Ok(item),
            _ => anyhow::bail!("Unexpected TL message"),
        }
    }
}

impl FromResponse for proto::SendMsgStatus {
    fn from_response(response: proto::Response) -> Result<Self> {
        match response {
            proto::Response::SendMsgStatus(item) => Ok(item),
            _ => anyhow::bail!("Unexpected TL message"),
        }
    }
}

impl FromResponse for proto::Error {
    fn from_response(response: proto::Response) -> Result<Self> {
        match response {
            proto::Response::Error(item) => Ok(item),
            _ => anyhow::bail!("Unexpected TL message"),
        }
    }
}
