// Copyright 2021 Red Hat, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::collections::{BTreeMap, HashMap};

use futures::stream::TryStreamExt;
use netlink_ethtool::{
    self, CoalesceAttr, EthoolAttr, EthtoolHandle, EthtoolHeader, FeatureAttr,
    FeatureBit, PauseAttr,
};
use netlink_generic;
use serde::{Deserialize, Serialize, Serializer};

use crate::NisporError;

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Clone, Default)]
pub struct EthtoolPauseInfo {
    rx: bool,
    tx: bool,
    auto_negotiate: bool,
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Clone)]
#[serde(rename_all = "snake_case")]
pub struct EthtoolFeatureInfo {
    #[serde(serialize_with = "ordered_map")]
    pub fixed: HashMap<String, bool>,
    #[serde(serialize_with = "ordered_map")]
    pub changeable: HashMap<String, bool>,
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Clone, Default)]
pub struct EthtoolCoalesceInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pkt_rate_high: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pkt_rate_low: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_sample_interval: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rx_max_frames: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rx_max_frames_high: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rx_max_frames_irq: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rx_max_frames_low: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rx_usecs: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rx_usecs_high: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rx_usecs_irq: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rx_usecs_low: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats_block_usecs: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_max_frames: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_max_frames_high: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_max_frames_irq: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_max_frames_low: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_usecs: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_usecs_high: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_usecs_irq: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_usecs_low: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_adaptive_rx: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_adaptive_tx: Option<bool>,
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Clone, Default)]
pub struct EthtoolInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pause: Option<EthtoolPauseInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub features: Option<EthtoolFeatureInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coalesce: Option<EthtoolCoalesceInfo>,
}

fn ordered_map<S>(
    value: &HashMap<String, bool>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let ordered: BTreeMap<_, _> = value.iter().collect();
    ordered.serialize(serializer)
}

pub(crate) async fn get_ethtool_infos(
) -> Result<HashMap<String, EthtoolInfo>, NisporError> {
    let mut infos: HashMap<String, EthtoolInfo> = HashMap::new();

    let family_id = get_ethtool_family_id().await?;

    let (connection, mut handle, _) =
        netlink_ethtool::new_connection(family_id).unwrap();

    tokio::spawn(connection);

    let mut pause_infos = dump_pause_infos(&mut handle).await?;
    let mut feature_infos = dump_feature_infos(&mut handle).await?;
    let mut coalesce_infos = dump_coalesce_infos(&mut handle).await?;

    for (iface_name, pause_info) in pause_infos.drain() {
        infos.insert(
            iface_name,
            EthtoolInfo {
                pause: Some(pause_info),
                ..Default::default()
            },
        );
    }

    for (iface_name, feature_info) in feature_infos.drain() {
        match infos.get_mut(&iface_name) {
            Some(ref mut info) => {
                info.features = Some(feature_info);
            }
            None => {
                infos.insert(
                    iface_name,
                    EthtoolInfo {
                        features: Some(feature_info),
                        ..Default::default()
                    },
                );
            }
        };
    }

    for (iface_name, coalesce_info) in coalesce_infos.drain() {
        match infos.get_mut(&iface_name) {
            Some(ref mut info) => {
                info.coalesce = Some(coalesce_info);
            }
            None => {
                infos.insert(
                    iface_name,
                    EthtoolInfo {
                        coalesce: Some(coalesce_info),
                        ..Default::default()
                    },
                );
            }
        };
    }

    Ok(infos)
}

async fn dump_pause_infos(
    handle: &mut EthtoolHandle,
) -> Result<HashMap<String, EthtoolPauseInfo>, NisporError> {
    let mut infos = HashMap::new();
    let mut pause_handle = handle.pause().get(None).execute();
    while let Some(ethtool_msg) = pause_handle.try_next().await? {
        if let EthoolAttr::Pause(nlas) = ethtool_msg.nlas {
            let mut iface_name = None;
            let mut pause_info = EthtoolPauseInfo::default();

            for nla in nlas {
                if let PauseAttr::Header(hdrs) = nla {
                    iface_name = get_iface_name_from_header(&hdrs);
                } else if let PauseAttr::AutoNeg(v) = nla {
                    pause_info.auto_negotiate = v
                } else if let PauseAttr::Rx(v) = nla {
                    pause_info.rx = v
                } else if let PauseAttr::Tx(v) = nla {
                    pause_info.tx = v
                }
            }
            if let Some(i) = iface_name {
                infos.insert(i, pause_info);
            }
        }
    }
    Ok(infos)
}

async fn dump_feature_infos(
    handle: &mut EthtoolHandle,
) -> Result<HashMap<String, EthtoolFeatureInfo>, NisporError> {
    let mut infos = HashMap::new();
    let mut feature_handle = handle.feature().get(None).execute();
    while let Some(ethtool_msg) = feature_handle.try_next().await? {
        if let EthoolAttr::Feature(nlas) = ethtool_msg.nlas {
            let mut iface_name = None;
            let mut fixed_features = HashMap::new();
            let mut changeable_features = HashMap::new();

            for nla in nlas {
                if let FeatureAttr::Header(hdrs) = nla {
                    iface_name = get_iface_name_from_header(&hdrs);
                } else if let FeatureAttr::Hw(feature_bits) = nla {
                    for feature_bit in feature_bits {
                        match feature_bit {
                            FeatureBit {
                                index: _,
                                name,
                                value: true,
                            } => {
                                changeable_features.insert(name, false);
                            }
                            FeatureBit {
                                index: _,
                                name,
                                value: false,
                            } => {
                                fixed_features.insert(name, false);
                            }
                        }
                    }
                } else if let FeatureAttr::Active(feature_bits) = nla {
                    for feature_bit in feature_bits {
                        if fixed_features.contains_key(&feature_bit.name) {
                            fixed_features.insert(feature_bit.name, true);
                        } else if changeable_features
                            .contains_key(&feature_bit.name)
                        {
                            changeable_features.insert(feature_bit.name, true);
                        }
                    }
                }
            }

            if let Some(i) = iface_name {
                infos.insert(
                    i.to_string(),
                    EthtoolFeatureInfo {
                        fixed: fixed_features,
                        changeable: changeable_features,
                    },
                );
            }
        }
    }

    Ok(infos)
}

async fn dump_coalesce_infos(
    handle: &mut EthtoolHandle,
) -> Result<HashMap<String, EthtoolCoalesceInfo>, NisporError> {
    let mut infos = HashMap::new();
    let mut coalesce_handle = handle.coalesce().get(None).execute();
    while let Some(ethtool_msg) = coalesce_handle.try_next().await? {
        if let EthoolAttr::Coalesce(nlas) = ethtool_msg.nlas {
            let mut iface_name = None;
            let mut coalesce_info = EthtoolCoalesceInfo::default();

            for nla in nlas {
                match nla {
                    CoalesceAttr::Header(hdrs) => {
                        iface_name = get_iface_name_from_header(&hdrs)
                    }
                    CoalesceAttr::RxUsecs(d) => {
                        coalesce_info.rx_usecs = Some(d)
                    }
                    CoalesceAttr::RxMaxFrames(d) => {
                        coalesce_info.rx_max_frames = Some(d)
                    }
                    CoalesceAttr::RxUsecsIrq(d) => {
                        coalesce_info.rx_usecs_irq = Some(d)
                    }
                    CoalesceAttr::RxMaxFramesIrq(d) => {
                        coalesce_info.rx_max_frames_irq = Some(d)
                    }
                    CoalesceAttr::TxUsecs(d) => {
                        coalesce_info.tx_usecs = Some(d)
                    }
                    CoalesceAttr::TxMaxFrames(d) => {
                        coalesce_info.tx_max_frames = Some(d)
                    }
                    CoalesceAttr::TxUsecsIrq(d) => {
                        coalesce_info.tx_usecs_irq = Some(d)
                    }
                    CoalesceAttr::TxMaxFramesIrq(d) => {
                        coalesce_info.tx_max_frames_irq = Some(d)
                    }
                    CoalesceAttr::StatsBlockUsecs(d) => {
                        coalesce_info.stats_block_usecs = Some(d)
                    }
                    CoalesceAttr::UseAdaptiveRx(d) => {
                        coalesce_info.use_adaptive_rx = Some(d)
                    }
                    CoalesceAttr::UseAdaptiveTx(d) => {
                        coalesce_info.use_adaptive_tx = Some(d)
                    }
                    CoalesceAttr::PktRateLow(d) => {
                        coalesce_info.pkt_rate_low = Some(d)
                    }
                    CoalesceAttr::RxUsecsLow(d) => {
                        coalesce_info.rx_usecs_low = Some(d)
                    }
                    CoalesceAttr::RxMaxFramesLow(d) => {
                        coalesce_info.rx_max_frames_low = Some(d)
                    }
                    CoalesceAttr::TxUsecsLow(d) => {
                        coalesce_info.tx_usecs_low = Some(d)
                    }
                    CoalesceAttr::TxMaxFramesLow(d) => {
                        coalesce_info.tx_max_frames_low = Some(d)
                    }
                    CoalesceAttr::PktRateHigh(d) => {
                        coalesce_info.pkt_rate_high = Some(d)
                    }
                    CoalesceAttr::RxUsecsHigh(d) => {
                        coalesce_info.rx_usecs_high = Some(d)
                    }
                    CoalesceAttr::RxMaxFramesHigh(d) => {
                        coalesce_info.rx_max_frames_high = Some(d)
                    }
                    CoalesceAttr::TxUsecsHigh(d) => {
                        coalesce_info.tx_usecs_high = Some(d)
                    }
                    CoalesceAttr::TxMaxFramesHigh(d) => {
                        coalesce_info.tx_max_frames_high = Some(d)
                    }
                    CoalesceAttr::RateSampleInterval(d) => {
                        coalesce_info.rate_sample_interval = Some(d)
                    }
                    _ => eprintln!("WARN: Unsupported CoalesceAttr {:?}", nla),
                }
            }
            if let Some(i) = iface_name {
                infos.insert(i, coalesce_info);
            }
        }
    }
    Ok(infos)
}

async fn get_ethtool_family_id() -> Result<u16, NisporError> {
    let (connection, mut handle, _) =
        netlink_generic::new_connection().unwrap();
    tokio::spawn(connection);

    Ok(handle.resolve_family_name("ethtool").await?)
}

fn get_iface_name_from_header(hdrs: &[EthtoolHeader]) -> Option<String> {
    for hdr in hdrs {
        if let EthtoolHeader::DevName(iface_name) = hdr {
            return Some(iface_name.to_string());
        }
    }
    None
}
