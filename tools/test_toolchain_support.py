"""Regression checks for paths inspected by the native-host verifiers."""

from pathlib import PurePosixPath, PureWindowsPath
import unittest

from toolchain_support import cache_directory


class CacheDirectoryTests(unittest.TestCase):
    def test_compact_windows_cache_matches_the_compiler_identity_vector(self):
        path = cache_directory(
            PureWindowsPath("C:/project with spaces/target"),
            "1.98.0",
            "88d9e12ae178fab0fb5cc050a94da85685d449ea",
            "x86_64-pc-windows-msvc",
            "1111111111111111-2222222222222222",
        )
        self.assertEqual(path, PureWindowsPath("C:/project with spaces/target/nestrs/01c90ba81b78b681"))

    def test_linux_retains_the_existing_full_identity_layout(self):
        path = cache_directory(PurePosixPath("/project/target"), "1.98.0", "commit", "x86_64-unknown-linux-gnu", "driver-bridge")
        self.assertEqual(path, PurePosixPath("/project/target/nestrs/1.98.0-commit-x86_64-unknown-linux-gnu/driver-bridge"))


if __name__ == "__main__":
    unittest.main()
