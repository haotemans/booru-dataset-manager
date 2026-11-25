"""Serve BDM's private ModelScope backup operations over local MCP stdio."""

from __future__ import annotations

import argparse
import asyncio
from contextlib import redirect_stdout
import os
from pathlib import Path
import sys
from threading import Lock
from typing import Any, Literal

from mcp.server.fastmcp import FastMCP

from modelscope_backup import (
    BackupError, DATABASE, DEFAULT_CONFIG, ModelScope, backup_roots, configured_dataset,
    configured_site, receive_latest, restore, save_settings, windows_token, write_backup,
)
from modelscope_catalog import run_request


def server(config, source, roots, destination, receive_to, shared_workspace, project):
    mcp = FastMCP('BDM ModelScope backup')
    transfer_lock = Lock()

    def token_for(dataset):
        return os.getenv('MODELSCOPE_TOKEN') or windows_token(dataset)

    def site_for():
        return os.getenv('MODELSCOPE_BACKUP_SITE') or configured_site(config) or 'modelscope.cn'

    @mcp.tool(structured_output=True)
    def bdm_modelscope_status() -> dict[str, str | bool | None]:
        """Show the local BDM dataset setting and whether a token and source catalog are available."""
        try:
            dataset = configured_dataset(config)
            return {
                'dataset_id': dataset,
                'site': site_for(),
                'token_available': bool(dataset and token_for(dataset)),
                'source_catalog': str(source / DATABASE),
                'source_ready': (source / DATABASE).is_file(),
                'restore_destination': str(destination),
                'restore_destination_exists': destination.exists(),
                'receive_directory': str(receive_to),
                'shared_catalog_workspace': str(shared_workspace),
            }
        except BackupError as error:
            return {'error': str(error)}

    @mcp.tool(structured_output=True)
    def bdm_modelscope_configure(dataset_id: str, site: Literal['modelscope.cn', 'modelscope.ai'] | None = None) -> dict[str, str | bool]:
        """Save the private dataset ID and optional ModelScope site in BDM config. Enter tokens locally."""
        if not transfer_lock.acquire(blocking=False):
            return {'error': 'A ModelScope operation is in progress.'}
        try:
            repo = dataset_id.strip()
            save_settings(config, repo, site=site)
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
            # MCP stdio reserves stdout for JSON-RPC, including during SDK setup and downloads.
            with redirect_stdout(sys.stderr):
                remote = ModelScope(dataset, 'master', token.strip(), site=site_for())
                if action == 'backup':
                    version = write_backup(source, backup_roots(source, roots), remote)
                    return {'status': 'verified', 'dataset_id': dataset, 'backup_id': version}
                if action in {'restore', 'receive'}:
                    if action == 'receive':
                        version, directory = receive_latest(receive_to, remote)
                    else:
                        version, directory = restore(destination, remote, backup_id), destination
                    return {'status': 'verified', 'dataset_id': dataset, 'backup_id': version,
                            'destination': str(directory), 'catalog': str(directory / DATABASE),
                            'library_root': str(directory / 'library'), 'project_root': str(project)}
            return {'status': 'verified', 'dataset_id': dataset, 'visibility': 'private'}
        except BackupError as error:
            return {'error': str(error)}
        except Exception as error:
            # Raw SDK errors can include signed URLs and request details.
            return {'error': f'ModelScope operation failed ({type(error).__name__}). Check access, paths and network.'}
        finally:
            transfer_lock.release()

    @mcp.tool(structured_output=True)
    async def bdm_modelscope_transfer(action: Literal['check', 'backup', 'restore', 'receive'],
                                      backup_id: str | None = None) -> dict[str, str]:
        """Check private dataset access, upload, restore to a specified directory, or receive the latest version.

        Run backup when BDM editing and download jobs are idle. Receive reuses an existing verified version
        and returns the absolute catalog, library_root and project_root for a BDM MCP connection scoped to it.
        """
        if backup_id is not None and action != 'restore':
            return {'error': 'backup_id is only used by restore.'}
        return await asyncio.to_thread(transfer, action, backup_id)

    def catalog(action, arguments):
        if not transfer_lock.acquire(blocking=False):
            return {'error': 'A ModelScope operation is in progress.'}
        try:
            with redirect_stdout(sys.stderr):
                return run_request(config, shared_workspace, action, arguments or {})
        except BackupError as error:
            return {'error': str(error)}
        except Exception as error:
            return {'error': f'ModelScope catalog operation failed ({type(error).__name__}).'}
        finally:
            transfer_lock.release()

    @mcp.tool(structured_output=True)
    async def bdm_modelscope_catalog(
        action: Literal['status', 'view', 'list', 'add', 'edit', 'import', 'delete', 'commit', 'pull',
                        'push', 'history', 'inspect', 'original', 'clear-cache', 'resolve', 'revert'],
        arguments: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        """Work with the Git-versioned private gallery. Pull downloads metadata, original fetches one file.

        add takes file (local original), optional name, labels, caption, collection and review.
        edit takes entry_id and any of those fields. import takes a catalog path and media_id.
        delete, original and clear-cache take entry_id. list takes optional query. commit takes message.
        resolve takes entry_id and side (local or remote).
        inspect and revert take commit_id. pull, push, history, view and status take no arguments.
        """
        return await asyncio.to_thread(catalog, action, arguments)

    return mcp


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    root = DEFAULT_CONFIG.parents[1]
    parser.add_argument('--config', type=Path, default=DEFAULT_CONFIG)
    parser.add_argument('--data', type=Path, default=root / 'data')
    parser.add_argument('--library', type=Path, action='append', default=[])
    parser.add_argument('--restore-to', type=Path, default=root / 'data-restored')
    parser.add_argument('--receive-to', type=Path, default=root / 'dist/modelscope-received')
    parser.add_argument('--shared-workspace', type=Path)
    args = parser.parse_args()
    source = args.data.absolute()
    workspace = args.shared_workspace or source / 'modelscope-catalog'
    server(args.config.absolute(), source, args.library, args.restore_to.resolve(),
           args.receive_to.resolve(), workspace.resolve(), root).run(transport='stdio')


if __name__ == '__main__':
    main()
