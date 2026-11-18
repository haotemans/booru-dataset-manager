"""Serve BDM's private ModelScope backup operations over local MCP stdio."""

from __future__ import annotations

import argparse
import asyncio
import os
from pathlib import Path
from threading import Lock
from typing import Literal

from mcp.server.fastmcp import FastMCP

from modelscope_backup import (
    BackupError, DATABASE, DEFAULT_CONFIG, ModelScope, configured_dataset,
    restore, save_settings, windows_token, write_backup,
)


def server(config, source, roots, destination):
    mcp = FastMCP('BDM ModelScope backup')
    transfer_lock = Lock()

    def token_for(dataset):
        return os.getenv('MODELSCOPE_TOKEN') or windows_token(dataset)

    @mcp.tool(structured_output=True)
    def bdm_modelscope_status() -> dict[str, str | bool | None]:
        """Show the local BDM dataset setting and whether a token and source catalog are available."""
        try:
            dataset = configured_dataset(config)
            return {
                'dataset_id': dataset,
                'token_available': bool(dataset and token_for(dataset)),
                'source_catalog': str(source / DATABASE),
                'source_ready': (source / DATABASE).is_file(),
                'restore_destination': str(destination),
                'restore_destination_exists': destination.exists(),
            }
        except BackupError as error:
            return {'error': str(error)}

    @mcp.tool(structured_output=True)
    def bdm_modelscope_configure(dataset_id: str) -> dict[str, str | bool]:
        """Save the private ModelScope dataset ID in BDM config. Enter tokens in a local terminal, never as tool arguments."""
        if not transfer_lock.acquire(blocking=False):
            return {'error': 'A ModelScope operation is in progress.'}
        try:
            repo = dataset_id.strip()
            save_settings(config, repo)
            return {'dataset_id': repo, 'saved': True}
        except BackupError as error:
            return {'error': str(error)}
        except OSError:
            return {'error': 'Could not save the BDM ModelScope configuration.'}
        finally:
            transfer_lock.release()

    def transfer(action, backup_id):
        if not transfer_lock.acquire(blocking=False):
            return {'error': 'A ModelScope operation is in progress.'}
        try:
            dataset = configured_dataset(config)
            if not dataset:
                raise BackupError('Set the dataset ID in BDM using bdm_modelscope_configure first.')
            if action == 'backup' and not (source / DATABASE).is_file():
                raise BackupError(f'Local catalog was not found: {source / DATABASE}')
            if action == 'restore' and destination.exists():
                raise BackupError(f'Restore requires a new destination directory: {destination}')
            token = token_for(dataset)
            if not token or not token.strip():
                raise BackupError('Save an Access Token using modelscope_backup.py --repo owner/name configure in a local terminal.')
            remote = ModelScope(dataset, 'master', token.strip())
            if action == 'backup':
                version = write_backup(source, roots, remote)
                return {'status': 'verified', 'dataset_id': dataset, 'backup_id': version}
            if action == 'restore':
                version = restore(destination, remote, backup_id)
                return {'status': 'verified', 'dataset_id': dataset, 'backup_id': version,
                        'destination': str(destination), 'catalog': str(destination / DATABASE)}
            return {'status': 'verified', 'dataset_id': dataset, 'visibility': 'private'}
        except BackupError as error:
            return {'error': str(error)}
        except Exception as error:
            # Raw SDK errors can include signed URLs and request details.
            return {'error': f'ModelScope operation failed ({type(error).__name__}). Check access, paths and network.'}
        finally:
            transfer_lock.release()

    @mcp.tool(structured_output=True)
    async def bdm_modelscope_transfer(action: Literal['check', 'backup', 'restore'],
                                      backup_id: str | None = None) -> dict[str, str]:
        """Check private dataset access, back up the configured BDM catalog, or restore to the configured new directory.

        Run backup only when BDM editing and download jobs are idle. Restore defaults to the latest verified snapshot.
        """
        if backup_id is not None and action != 'restore':
            return {'error': 'backup_id is only used by restore.'}
        return await asyncio.to_thread(transfer, action, backup_id)

    return mcp


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    root = DEFAULT_CONFIG.parents[1]
    parser.add_argument('--config', type=Path, default=DEFAULT_CONFIG)
    parser.add_argument('--data', type=Path, default=root / 'data')
    parser.add_argument('--library', type=Path, action='append', default=[])
    parser.add_argument('--restore-to', type=Path, default=root / 'data-restored')
    args = parser.parse_args()
    source = args.data.absolute()
    roots = list(dict.fromkeys(path.absolute() for path in [source / 'library', *args.library]))
    server(args.config.absolute(), source, roots, args.restore_to.resolve()).run(transport='stdio')


if __name__ == '__main__':
    main()
