use {
    std::{net::SocketAddr, path::PathBuf},
    tokio::runtime::Runtime,
};

use crate::{
    disk::{self, Alert},
    error::*,
    metainfo::Metainfo,
    storage_info::StorageInfo,
    torrent::Torrent,
    PeerId,
};

/// Connects to a single seed and downloads the torrent or aborts on error.
pub fn run_torrent(
    client_id: PeerId,
    download_dir: PathBuf,
    metainfo: Metainfo,
    seeds: Vec<SocketAddr>,
) -> Result<()> {
    if seeds.is_empty() {
        log::warn!("List of seeds is empty, cannot download torrent");
        return Ok(());
    }

    let mut rt = Runtime::new()?;
    rt.block_on(async move {
        start_disk_and_torrent(client_id, download_dir, metainfo, seeds).await
    })
}

async fn start_disk_and_torrent(
    client_id: PeerId,
    download_dir: PathBuf,
    metainfo: Metainfo,
    seeds: Vec<SocketAddr>,
) -> Result<()> {
    let (disk_join_handle, disk, mut alert_port) = disk::spawn()?;

    // allocate torrent on disk
    let id = 0;
    let info_hash = metainfo.info_hash;
    let storage_info = StorageInfo::new(&metainfo, download_dir);
    log::info!("Torrent {} storage info: {:?}", id, storage_info);

    // allocate torrent and wait for its result
    disk.allocate_new_torrent(id, storage_info.clone(), metainfo.pieces)?;
    let torrent_disk_alert_port =
        if let Some(Alert::TorrentAllocation(allocation_result)) =
            alert_port.recv().await
        {
            match allocation_result {
                Ok(allocation) => {
                    log::info!("Torrent {} allocated on disk", id);
                    debug_assert_eq!(allocation.id, id);
                    allocation.alert_port
                }
                Err(e) => {
                    log::error!(
                        "Torrent {} could not be allocated on disk: {}",
                        id,
                        e
                    );
                    return Ok(());
                }
            }
        } else {
            log::error!(
                "Disk task receive error, disk task most likely stopped"
            );
            return Ok(());
        };

    let mut torrent = Torrent::new(
        id,
        disk.clone(),
        torrent_disk_alert_port,
        info_hash,
        storage_info,
        client_id,
        &seeds,
    )?;
    // run torrent to completion
    torrent.start().await?;

    // send a shutdown command to disk
    disk.shutdown()?;
    // and join on its handle
    disk_join_handle
        .await
        .expect("Disk task has panicked")
        .map_err(Error::from)?;

    Ok(())
}
