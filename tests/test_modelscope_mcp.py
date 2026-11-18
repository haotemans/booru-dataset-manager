import json
import logging
from pathlib import Path
import sqlite3
import sys
from tempfile import TemporaryDirectory
import unittest

try:
    from mcp import ClientSession, StdioServerParameters
    from mcp.client.stdio import stdio_client
except ModuleNotFoundError:
    ClientSession = None


PROJECT = Path(__file__).resolve().parents[1]


@unittest.skipIf(ClientSession is None, 'Optional MCP SDK is required')
class ModelScopeMcpTests(unittest.IsolatedAsyncioTestCase):
    async def test_configure_backup_and_restore_over_stdio(self):
        with TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'source'
            source.mkdir()
            db = sqlite3.connect(source / 'catalog.sqlite3')
            db.execute('CREATE TABLE media (id INTEGER PRIMARY KEY, metadata TEXT NOT NULL)')
            db.close()
            config = root / 'modelscope.json'
            destination = root / 'restored'
            fixture = '''
import sys
sys.path[:0] = ['scripts', 'tests']
import modelscope_mcp
from test_modelscope_backup import MemoryDataset
remote = MemoryDataset()
upload = remote.upload
def verbose_upload(*args):
    print('SDK commit progress', flush=True)
    return upload(*args)
remote.upload = verbose_upload
modelscope_mcp.ModelScope = lambda *args: remote
modelscope_mcp.main()
'''
            entry = StdioServerParameters(
                command=sys.executable, cwd=PROJECT,
                args=['-c', fixture, '--config', str(config), '--data', str(source),
                      '--restore-to', str(destination)],
                env={'MODELSCOPE_TOKEN': 'synthetic-mcp-test-token'},
            )
            async with stdio_client(entry) as (read, write), ClientSession(read, write) as session:
                await session.initialize()
                listed = await session.list_tools()
                self.assertEqual({tool.name for tool in listed.tools}, {
                    'bdm_modelscope_status', 'bdm_modelscope_configure', 'bdm_modelscope_transfer',
                })
                configured = await session.call_tool('bdm_modelscope_configure', {'dataset_id': 'owner/data'})
                self.assertTrue(configured.structuredContent['saved'])
                self.assertEqual(json.loads(config.read_text()), {'dataset_id': 'owner/data'})
                status = await session.call_tool('bdm_modelscope_status', {})
                self.assertTrue(status.structuredContent['token_available'])
                self.assertNotIn('synthetic-mcp-test-token', str(status))
                with self.assertNoLogs('mcp.client.stdio', level=logging.ERROR):
                    backup = await session.call_tool('bdm_modelscope_transfer', {'action': 'backup'})
                self.assertEqual(backup.structuredContent['status'], 'verified')
                restored = await session.call_tool('bdm_modelscope_transfer', {'action': 'restore'})
                self.assertEqual(restored.structuredContent['backup_id'], backup.structuredContent['backup_id'])
                self.assertTrue((destination / 'catalog.sqlite3').is_file())
                self.assertTrue((destination / 'library').is_dir())
