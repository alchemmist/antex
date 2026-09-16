from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from antex_package import v8


class DownloadFileTest(unittest.TestCase):
    @patch.object(
        v8,
        "resolve_github_asset_url",
        return_value="https://release-assets.githubusercontent.com/asset",
    )
    def test_github_release_prefers_ipv6_asset_download(
        self,
        resolve_url,
    ) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            destination = Path(temp_dir) / "artifact"

            def download_over_ipv6(_url: str, temp_path: Path) -> bool:
                temp_path.write_bytes(b"artifact")
                return True

            with (
                patch.object(
                    v8,
                    "download_github_asset_over_ipv6",
                    side_effect=download_over_ipv6,
                ) as download,
                patch.object(v8, "run_curl") as run_curl,
            ):
                v8.download_file(
                    "https://github.com/org/repo/releases/asset", destination
                )

            self.assertEqual(destination.read_bytes(), b"artifact")

        resolve_url.assert_called_once()
        download.assert_called_once()
        run_curl.assert_not_called()

    @patch.object(v8, "run_curl")
    def test_resolve_github_asset_url_accepts_release_asset_redirect(
        self,
        run_curl,
    ) -> None:
        run_curl.return_value = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout="HTTP/2 302\nLocation: https://release-assets.githubusercontent.com/asset\n",
            stderr="",
        )

        self.assertEqual(
            v8.resolve_github_asset_url("https://github.com/org/repo/releases/asset"),
            "https://release-assets.githubusercontent.com/asset",
        )

    @patch.object(v8, "run_curl")
    def test_resolve_github_asset_url_rejects_untrusted_redirect(
        self, run_curl
    ) -> None:
        run_curl.return_value = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout="HTTP/2 302\nLocation: https://example.com/asset\n",
            stderr="",
        )

        with self.assertRaisesRegex(RuntimeError, "unexpected release redirect"):
            v8.resolve_github_asset_url("https://github.com/org/repo/releases/asset")


if __name__ == "__main__":
    unittest.main()
