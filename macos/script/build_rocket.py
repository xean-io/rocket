"""SwiftPM bundle assembly and owned-process verification. macOS only."""
import argparse
import ctypes
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import urllib.request
import uuid

MACOS = Path(__file__).resolve().parents[1]
REPO = MACOS.parent
BUILD = MACOS / "build"


class SafetyError(RuntimeError):
    pass


class BSDInfo(ctypes.Structure):
    # sys/proc_info.h: PROC_PIDTBSDINFO, MAXCOMLEN=16.
    _fields_ = [(name, ctypes.c_uint32) for name in (
        "flags", "status", "xstatus", "pid", "ppid", "uid", "gid", "ruid", "rgid", "svuid", "svgid", "reserved"
    )] + [("comm", ctypes.c_char * 16), ("name", ctypes.c_char * 32)] + [
        (name, ctypes.c_uint32) for name in ("nfiles", "pgid", "pjobc", "tdev", "tpgid")
    ] + [("nice", ctypes.c_int32), ("start_sec", ctypes.c_uint64), ("start_usec", ctypes.c_uint64)]


def process_identity(pid):
    lib = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
    lib.proc_pidpath.argtypes = [ctypes.c_int, ctypes.c_void_p, ctypes.c_uint32]
    lib.proc_pidinfo.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_uint64, ctypes.c_void_p, ctypes.c_int]
    info = BSDInfo()
    if lib.proc_pidinfo(pid, 3, 0, ctypes.byref(info), ctypes.sizeof(info)) != ctypes.sizeof(info):
        # Distinguish an exited PID from unreadable live metadata; fail closed.
        result = subprocess.run(["/bin/ps", "-p", str(pid), "-o", "pid="], capture_output=True, text=True)
        if result.stdout.strip():
            raise SafetyError(f"Cannot establish start-time identity for live PID {pid}")
        return None
    if info.status == 5:  # SZOMB: not a live process.
        return None
    path = ctypes.create_string_buffer(4096)
    if lib.proc_pidpath(pid, path, len(path)) <= 0:
        raise SafetyError(f"Cannot establish executable identity for PID {pid}")
    return {"pid": pid, "executable": str(Path(os.fsdecode(path.value)).resolve()), "uid": info.uid,
            "start_sec": info.start_sec, "start_usec": info.start_usec}


def matching_processes(executable):
    target = str(executable.resolve())
    pids = subprocess.check_output(["/bin/ps", "-axo", "pid="], text=True).split()
    matches = []
    for raw in pids:
        try:
            identity = process_identity(int(raw))
        except SafetyError:
            continue  # Unrelated processes need not be inspectable.
        if identity and identity["executable"] == target:
            matches.append(identity)
    return matches


def assert_identity(expected):
    if expected is None:
        return None
    current = process_identity(expected["pid"])
    if current is not None and (current != expected or current["uid"] != os.getuid()):
        raise SafetyError(f"Receipt identity mismatch for PID {expected['pid']}; no signal sent")
    return current


def stop_owned(expected):
    if assert_identity(expected) is None:
        return
    os.kill(expected["pid"], signal.SIGTERM)
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if assert_identity(expected) is None:
            return
        time.sleep(0.1)
    if assert_identity(expected) is not None:
        os.kill(expected["pid"], signal.SIGKILL)
    deadline = time.monotonic() + 3
    while time.monotonic() < deadline:
        if assert_identity(expected) is None:
            return
        time.sleep(0.1)
    raise SafetyError(f"Owned PID {expected['pid']} did not exit")


def save_receipt(path, receipt):
    path.write_text(json.dumps(receipt, indent=2) + "\n")
    path.chmod(0o600)


def private_home(path):
    raw = str(path)
    if not re.fullmatch(r"/tmp/rkv-[A-Za-z0-9_]{6,24}", raw):
        raise SafetyError("Verification home must be an explicit short /tmp/rkv-* directory")
    if str(path.resolve()) not in (raw, "/private" + raw):
        raise SafetyError("Verification home resolves outside its private directory")
    attrs = path.stat()
    if not path.is_dir() or attrs.st_uid != os.getuid() or attrs.st_mode & 0o777 != 0o700:
        raise SafetyError("Verification home must be an owned 0700 directory")


def api(receipt, path, body=None):
    config = json.loads((Path(receipt["home"]) / "daemon.json").read_text())
    if config["pid"] != receipt["daemon"]["pid"]:
        raise SafetyError("Private daemon.json no longer identifies the receipt-owned daemon")
    if assert_identity(receipt["daemon"]) is None:
        raise SafetyError("Receipt-owned daemon has exited")
    data = None if body is None else json.dumps(body).encode()
    request = urllib.request.Request(config["http"] + path, data=data, headers={
        "Authorization": "Bearer " + config["token"], "Content-Type": "application/json"
    })
    with urllib.request.urlopen(request, timeout=20) as response:
        return json.load(response)


def wait_daemon_files_gone(home, timeout=5):
    paths = [home / "daemon.json", home / "rocketd.sock"]
    deadline = time.monotonic() + timeout
    while any(path.exists() for path in paths):
        if time.monotonic() >= deadline:
            raise SafetyError("Owned daemon exited but daemon.json or unix socket remains")
        time.sleep(0.1)


def cleanup(receipt_path):
    receipt_path = Path(receipt_path).resolve()
    if not receipt_path.is_relative_to(BUILD.resolve()):
        raise SafetyError("Receipt must be inside Rocket's macOS build directory")
    attrs = receipt_path.stat()
    if attrs.st_uid != os.getuid() or attrs.st_mode & 0o077:
        raise SafetyError("Receipt is not private and owned")
    receipt = json.loads(receipt_path.read_text())
    bundle = Path(receipt["bundle"]).resolve()
    if not bundle.is_relative_to(BUILD.resolve()):
        raise SafetyError("Receipt bundle lies outside Rocket's build directory")
    app = receipt.get("app")
    if app and app["executable"] != str(bundle / "Contents/MacOS/Rocket"):
        raise SafetyError("Receipt app executable does not match its bundle")
    if receipt["verification"]:
        private_home(Path(receipt["home"]))
        if not bundle.is_relative_to((BUILD / "verification").resolve()):
            raise SafetyError("Verification receipt points at a habitual bundle")
        daemon = receipt.get("daemon")
        if daemon and daemon["executable"] != str(bundle.parent / "rocket"):
            raise SafetyError("Receipt daemon executable does not match its private binary")
        assert_identity(daemon)
    # Validate all identities before changing anything, then stop app before daemon.
    assert_identity(app)
    stop_owned(app)
    if receipt["verification"] and assert_identity(receipt.get("daemon")) is not None:
        runs = api(receipt, "/v1/ps?all=true").get("services") or []
        jobs = api(receipt, "/v1/jobs?all=true").get("jobs") or []
        active = {"starting", "running", "stopping"}
        children = [process_identity(run["pid"]) for run in runs if run.get("pid") and run["state"] in active]
        children += [process_identity(job["pid"]) for job in jobs if job.get("pid") and job["status"] == "running"]
        result = api(receipt, "/v1/down", {"everywhere": True})
        if result.get("errors"):
            raise SafetyError("Owned fixture shutdown failed: " + "; ".join(result["errors"]))
        if any(run["state"] in active for run in api(receipt, "/v1/ps?all=true").get("services") or []):
            raise SafetyError("Owned fixture services are still active")
        if api(receipt, "/v1/ports").get("ports"):
            raise SafetyError("Owned fixture port leases remain")
        if any(job["status"] == "running" for job in api(receipt, "/v1/jobs?all=true").get("jobs") or []):
            raise SafetyError("Owned fixture jobs are still running")
        for child in children:
            if child and process_identity(child["pid"]) == child:
                raise SafetyError(f"Owned fixture child PID {child['pid']} remains")
        stop_owned(receipt["daemon"])
    if receipt["verification"]:
        wait_daemon_files_gone(Path(receipt["home"]))
    receipt["cleaned"] = True
    save_receipt(receipt_path, receipt)
    print(f"cleaned receipt-owned processes: {receipt_path}", flush=True)


def build_bundle(config, directory, display_name, bundle_id, verification):
    bundle = directory / (display_name + ".app")
    executable = bundle / "Contents/MacOS/Rocket"
    if matching_processes(executable):
        raise SafetyError("Cannot replace a running app bundle. Use its matching receipt to stop it first.")
    subprocess.run(["swift", "build", "-c", config, "--product", "Rocket"], cwd=MACOS, check=True)
    bin_dir = Path(subprocess.check_output(["swift", "build", "-c", config, "--show-bin-path"], cwd=MACOS, text=True).strip())
    if matching_processes(executable):
        raise SafetyError("An app opened during the build; its running bundle will not be replaced")
    if bundle.exists():
        shutil.rmtree(bundle)
    (bundle / "Contents/MacOS").mkdir(parents=True)
    resources = bundle / "Contents/Resources"
    resources.mkdir()
    shutil.copy2(bin_dir / "Rocket", executable)
    info = {"CFBundleDevelopmentRegion": "en", "CFBundleDisplayName": display_name, "CFBundleExecutable": "Rocket",
            "CFBundleIdentifier": bundle_id, "CFBundleInfoDictionaryVersion": "6.0", "CFBundleName": display_name,
            "CFBundlePackageType": "APPL", "CFBundleShortVersionString": "0.1.0", "CFBundleVersion": "1",
            "CFBundleIconFile": "AppIcon", "LSApplicationCategoryType": "public.app-category.developer-tools",
            "LSMinimumSystemVersion": "26.0", "LSUIElement": False, "NSHighResolutionCapable": True,
            "NSPrincipalClass": "NSApplication", "NSSupportsAutomaticTermination": False,
            "NSAppTransportSecurity": {"NSAllowsLocalNetworking": True}, "RocketVerificationBundle": verification}
    if subprocess.run(["xcrun", "--find", "actool"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0:
        partial = directory / "icon-partial.plist"
        subprocess.run(["xcrun", "actool", "--compile", str(resources), "--app-icon", "AppIcon", "--accent-color", "AccentColor",
                        "--platform", "macosx", "--minimum-deployment-target", "26.0", "--target-device", "mac",
                        "--output-partial-info-plist", str(partial), "--errors", "--warnings",
                        str(MACOS / "Resources/Assets.xcassets"), str(MACOS / "Resources/AppIcon.icon")], check=True,
                       stdout=subprocess.DEVNULL)
        info.update(plistlib.loads(partial.read_bytes()))
        partial.unlink()
    else:
        shutil.copy2(MACOS / "Resources/AppIcon.icns", resources / "AppIcon.icns")
    info["RocketVerificationBundle"] = verification
    (bundle / "Contents/Info.plist").write_bytes(plistlib.dumps(info))
    subprocess.run(["codesign", "--force", "--sign", "-", "--timestamp=none", str(bundle)], check=True,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    print(f"built {bundle} ({config})", flush=True)
    return bundle


def launch(bundle, environment, appearance=None):
    command = ["/usr/bin/open", "-n"]
    for key, value in environment.items():
        command.extend(["--env", key + "=" + str(value)])
    command.append(str(bundle))
    if appearance:
        command.extend(["--args", "--verification-appearance", appearance])
    subprocess.run(command, check=True)
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        matches = matching_processes(bundle / "Contents/MacOS/Rocket")
        if len(matches) == 1:
            return matches[0]
        if len(matches) > 1:
            raise SafetyError("More than one process uses this bundle; cannot establish ownership")
        time.sleep(0.2)
    raise SafetyError("The new bundle did not remain running")


def verify(args, bundle):
    directory = bundle.parent
    home = Path(tempfile.mkdtemp(prefix="rkv-", dir="/tmp"))
    private_home(home)
    rocket = directory / "rocket"
    supplied = os.environ.get("ROCKET_BIN")
    if supplied:
        shutil.copy2(Path(supplied).resolve(), rocket)
    else:
        subprocess.run(["go", "build", "-o", str(rocket), "./cmd/rocket"], cwd=REPO, check=True)
    fixture = directory / "fixture"
    fixture.mkdir()
    source = Path(args.fixture).resolve() if args.fixture else Path(__file__).with_name("fixtures") / "verification.yaml"
    if not source.is_relative_to(REPO):
        raise SafetyError("Verification fixture must be inside Rocket")
    shutil.copy2(source, fixture / "rocket.yaml")
    # Explicit LaunchServices environment, not shell inheritance.
    environment = {"ROCKET_HOME": str(home), "ROCKET_BIN": str(rocket)}
    cli_env = dict(os.environ, **environment)
    receipt_path = directory / "receipt.json"
    receipt = {"verification": True, "bundle": str(bundle), "home": str(home), "fixture": str(fixture),
               "app": None, "daemon": None, "cleaned": False}
    save_receipt(receipt_path, receipt)
    success = False
    try:
        started_daemon = subprocess.run([str(rocket), "daemon", "start"], env=cli_env, stdout=subprocess.DEVNULL, timeout=30)
        info_path = home / "daemon.json"
        if info_path.exists():
            info = json.loads(info_path.read_text())
            receipt["daemon"] = process_identity(info["pid"])
        else:
            matches = matching_processes(rocket)
            if len(matches) == 1:
                receipt["daemon"] = matches[0]
        save_receipt(receipt_path, receipt)
        started_daemon.check_returncode()
        if receipt["daemon"] is None or receipt["daemon"]["executable"] != str(rocket.resolve()):
            raise SafetyError("Cannot establish ownership of the new daemon")
        save_receipt(receipt_path, receipt)
        subprocess.run([str(rocket), "projects", "add", str(fixture)], env=cli_env, check=True, stdout=subprocess.DEVNULL, timeout=15)
        started = subprocess.run([str(rocket), "-p", str(fixture), "up", "preview", "--owner", "agent:rocket-verification",
                                  "--ttl", "30m", "--json"], env=cli_env, check=True, capture_output=True, text=True, timeout=45)
        (directory / "up.json").write_text(started.stdout)
        receipt["app"] = launch(bundle, environment, args.appearance)
        save_receipt(receipt_path, receipt)
        report_path = directory / "app-ready.json"
        deadline = time.monotonic() + 40
        next_event = 0
        while time.monotonic() < deadline:
            if assert_identity(receipt["app"]) is None:
                raise SafetyError("The verification app exited")
            if report_path.exists():
                report = json.loads(report_path.read_text())
                if (report["app_pid"] == receipt["app"]["pid"] and report["daemon_pid"] == receipt["daemon"]["pid"]
                        and report["home"] == str(home) and report["sse_events"] > 0):
                    success = True
                    print(json.dumps({"bundle": str(bundle), "receipt": str(receipt_path), "home": str(home),
                                      "fixture": str(fixture / "rocket.yaml"), "app": receipt["app"],
                                      "daemon": receipt["daemon"], "sse_events": report["sse_events"]}, indent=2), flush=True)
                    break
            if time.monotonic() >= next_event:
                subprocess.run([str(rocket), "-p", str(fixture), "run", "check", "--detach", "--ttl", "1m"],
                               env=cli_env, check=True, stdout=subprocess.DEVNULL, timeout=15)
                next_event = time.monotonic() + 2
            time.sleep(0.2)
        if not success:
            raise SafetyError("Verification app did not confirm its private daemon and SSE events")
    except BaseException:
        try:
            cleanup(receipt_path)
        except Exception as cleanup_error:
            print(f"Cleanup also failed; retained receipt {receipt_path}: {cleanup_error}", file=sys.stderr, flush=True)
        raise
    else:
        if not args.keep_running:
            cleanup(receipt_path)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--debug", action="store_true")
    modes = parser.add_mutually_exclusive_group()
    for flag in ("run", "verify", "logs", "self-check"):
        modes.add_argument("--" + flag, action="store_true")
    modes.add_argument("--cleanup", type=Path, metavar="RECEIPT")
    parser.add_argument("--keep-running", action="store_true", help="Keep only a successful verification launch for UI inspection")
    parser.add_argument("--appearance", choices=("light", "dark"), help="Appearance of this verification instance only")
    parser.add_argument("--fixture", help="Rocket-local harmless verification manifest, copied into the isolated fixture")
    args = parser.parse_args()
    if args.cleanup:
        cleanup(args.cleanup)
        return
    if (args.keep_running or args.appearance or args.fixture) and not args.verify:
        parser.error("--keep-running, --appearance and --fixture require --verify")
    config = "debug" if args.debug else "release"
    normal_receipt = BUILD / "run-receipt.json"
    if args.verify:
        identifier = uuid.uuid4().hex[:8]
        directory = BUILD / "verification" / identifier
        directory.mkdir(parents=True)
        bundle = build_bundle(config, directory, "Rocket Verify " + identifier,
                              "com.xean.rocket.verify.v" + identifier, True)
        verify(args, bundle)
        return
    if (args.run or args.logs) and normal_receipt.exists():
        cleanup(normal_receipt)
    BUILD.mkdir(parents=True, exist_ok=True)
    bundle = build_bundle(config, BUILD, "Rocket", "com.xean.rocket", False)
    if args.self_check:
        subprocess.run([str(bundle / "Contents/MacOS/Rocket"), "--self-check"], check=True)
    elif args.run or args.logs:
        environment = {key: os.environ[key] for key in ("ROCKET_HOME", "ROCKET_BIN") if os.environ.get(key)}
        app = launch(bundle, environment)
        save_receipt(normal_receipt, {"verification": False, "bundle": str(bundle), "app": app, "cleaned": False})
        print(f"app PID {app['pid']}; receipt {normal_receipt}", flush=True)
        if args.logs:
            subprocess.run(["/usr/bin/log", "stream", "--info", "--style", "compact", "--predicate",
                            f"processIdentifier == {app['pid']}"], check=True)


if __name__ == "__main__":
    try:
        main()
    except (SafetyError, subprocess.CalledProcessError, subprocess.TimeoutExpired, OSError, ValueError) as error:
        raise SystemExit(str(error))
