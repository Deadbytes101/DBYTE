from pathlib import Path
import hashlib, json, socket, subprocess, time, uuid

from v13_release_identity import FROZEN_BASE_HEAD, expected_head

REPO = Path(__file__).resolve().parent.parent
COMPILER = REPO / "target" / "debug" / "dbytec.exe"
QEMU = Path(r"C:\Program Files\qemu\qemu-system-x86_64.exe")
EXPECTED_HEAD = expected_head()
ROOT = REPO / "tmp" / "v13_g4" / ("input-" + uuid.uuid4().hex)
ROOT.mkdir(parents=True)
COMMAND_INDEX = 0

def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest().upper()

def run(args, description, cwd=REPO):
    global COMMAND_INDEX
    COMMAND_INDEX += 1
    p = subprocess.run([str(x) for x in args], cwd=cwd, text=True,
                       stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    (ROOT / f"command-{COMMAND_INDEX:03d}.txt").write_text(
        f"CWD={cwd}\nCOMMAND={' '.join(map(str,args))}\nEXIT={p.returncode}\n{p.stdout}", encoding="utf-8")
    if p.returncode:
        raise RuntimeError(f"{description} failed ({p.returncode})")
    return p.stdout
def wslpath(path):
    return run(["wsl.exe","-d","Debian","--","wslpath","-a","-u",str(path).replace('\\','/')],
               "translate WSL path").strip()

def assert_true(value, message):
    if not value:
        raise RuntimeError(message)

def source_guard():
    input_src=(REPO/"examples/native_v13_machine/input_ps2.dby").read_text(encoding="utf-8")
    assert_true("0xd2" not in input_src.lower(), "forbidden i8042 D2 injection present")
    assert_true("COMMAND_WRITE_OUTPUT" not in input_src, "forbidden output injection symbol present")
    assert_true("keyboard_inject" not in input_src.lower(), "guest keyboard injection helper present")
    assert_true("interrupt_address(65, irq1)" in input_src, "IRQ1 gate binding missing")
    assert_true("apic.route_keyboard_irq()" in input_src, "IOAPIC keyboard route missing")

def source_manifest():
    paths=run(["git.exe","ls-files","--cached","--others","--exclude-standard"],"source enumeration").splitlines()
    rows=[]
    for rel in sorted(set(paths)):
        if rel.startswith(("crates/","runtime/","examples/native_v13_machine/","scripts/")) or rel.startswith("Cargo."):
            p=REPO/rel
            if p.is_file(): rows.append({"path":rel.replace('\\','/'),"sha256":sha(p)})
    return rows
def build_set(label,opt,date):
    out=ROOT/f"{label}-o{opt}"; out.mkdir()
    obj=out/"kernel.o"; elf=out/"kernel.elf"
    base=[COMPILER,"build","examples/native_v13_machine/kernel_g4_input.dby","--target","x86_64-dbyte-none","--opt-level",opt]
    run(base+["--emit","obj","-o",obj],f"build input {label} O{opt} object")
    run(base+["--emit","elf","-o",elf],f"build input {label} O{opt} ELF")
    run(["wsl.exe","-d","Debian","--","grub-file","--is-x86-multiboot2",wslpath(elf)],"grub-file")
    tree=out/"iso"; grub=tree/"boot"/"grub"; grub.mkdir(parents=True)
    (tree/"boot"/"kernel.elf").write_bytes(elf.read_bytes())
    (grub/"grub.cfg").write_text("set timeout=0\nset default=0\nmenuentry 'DByte v13 G4 Input' {\n multiboot2 /boot/kernel.elf\n boot\n}\n",encoding="ascii")
    iso=out/"input.iso"
    run(["wsl.exe","-d","Debian","--","grub-mkrescue","-o",wslpath(iso),f"--modification-date={date}","--set_all_file_dates",date,wslpath(tree)],"deterministic input ISO")
    return {"label":label,"opt":opt,"obj":str(obj),"elf":str(elf),"iso":str(iso),
            "obj_sha256":sha(obj),"elf_sha256":sha(elf),"iso_sha256":sha(iso),"iso_bytes":iso.stat().st_size}

def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1",0)); return s.getsockname()[1]
def qmp_connect(port,process):
    deadline=time.time()+10
    while time.time()<deadline:
        try:
            s=socket.create_connection(("127.0.0.1",port),timeout=1)
            f=s.makefile("rwb",buffering=0)
            json.loads(f.readline())
            return s,f
        except OSError:
            if process.poll() is not None: raise RuntimeError("QEMU exited before QMP connect")
            time.sleep(.05)
    raise RuntimeError("QMP connect timeout")

def qmp_cmd(f,name,args=None,ident=1):
    req={"execute":name,"id":ident}
    if args is not None: req["arguments"]=args
    f.write((json.dumps(req,separators=(",",":"))+"\n").encode())
    while True:
        line=f.readline()
        if not line: raise RuntimeError("QMP closed")
        obj=json.loads(line)
        if obj.get("id")==ident:
            if "error" in obj: raise RuntimeError(f"QMP {name} error: {obj['error']}")
            return obj.get("return")

def read_shared(path):
    try: return Path(path).read_text(encoding="ascii",errors="replace")
    except (FileNotFoundError,PermissionError): return ""
def wait_serial(serial,needle,process,timeout):
    deadline=time.time()+timeout
    while time.time()<deadline:
        text=read_shared(serial)
        if needle in text: return text
        if process.poll() is not None: break
        time.sleep(.05)
    raise RuntimeError(f"timeout waiting for {needle!r}; qemu={process.poll()}; serial={read_shared(serial)}")

def assert_trace(trace,entry_resets):
    text=read_shared(trace)
    assert_true("Triple fault" not in text,"triple fault observed")
    for token in ("v=06","v=0d","v=0e"):
        assert_true(token not in text,f"CPU fault observed: {token}")
    final_resets=text.count("CPU Reset")
    assert_true(final_resets==entry_resets,"CPU reset after kernel entry")
    return final_resets

def qemu_args(iso,ram,serial,trace,port):
    return [str(QEMU),"-cdrom",iso,"-boot","order=d","-m",f"{ram}M","-display","none",
            "-monitor","none","-serial",f"file:{serial}","-d","int,cpu_reset,guest_errors",
            "-D",str(trace),"-no-reboot","-no-shutdown",
            "-qmp",f"tcp:127.0.0.1:{port},server=on,wait=off"]
def boot_positive(build,ram):
    out=ROOT/f"run-o{build['opt']}-{ram}m-positive"; out.mkdir()
    serial=out/"serial.log"; trace=out/"trace.log"; port=free_port()
    args=qemu_args(build["iso"],ram,serial,trace,port)
    (out/"qemu-command.txt").write_text(" ".join(args),encoding="utf-8")
    p=subprocess.Popen(args,cwd=REPO,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE,text=True)
    s=f=None
    try:
        s,f=qmp_connect(port,p); qmp_cmd(f,"qmp_capabilities",ident=10)
        text=wait_serial(serial,"INPUT READY gsi=",p,30)
        entry_resets=read_shared(trace).count("CPU Reset")
        sequence=[("a",True,"00","1E"),("a",False,"01","9E"),("b",True,"02","30"),("b",False,"03","B0"),
                  ("c",True,"04","2E"),("c",False,"05","AE"),("d",True,"06","20"),("d",False,"07","A0")]
        for ident,(key,down,index,scan) in enumerate(sequence,20):
            qmp_cmd(f,"input-send-event",{"events":[{"type":"key","data":{"down":down,"key":{"type":"qcode","data":key}}}]},ident)
            text=wait_serial(serial,f"INPUT ACK index={index} scan={scan}",p,5)
        text=wait_serial(serial,"G4 INPUT OBSERVED COMPLETE",p,10)
        assert_true("INPUT COMPLETE irq=00000008 eoi=00000008 pushes=00000008 drops=00000000 malformed=00000000 timer=" in text,"input counters mismatch")
        final_resets=assert_trace(trace,entry_resets)
        return {"opt":build["opt"],"ram":ram,"kind":"positive","serial_sha256":sha(serial),"trace_sha256":sha(trace),"entry_resets":entry_resets,"final_resets":final_resets}
    finally:
        if f:
            try:qmp_cmd(f,"quit",ident=99)
            except Exception: pass
        if s:
            try:s.close()
            except Exception:pass
        try:p.wait(timeout=3)
        except subprocess.TimeoutExpired: p.kill(); p.wait()
def boot_timeout(build):
    out=ROOT/f"run-o{build['opt']}-64m-timeout"; out.mkdir()
    serial=out/"serial.log"; trace=out/"trace.log"; port=free_port()
    args=qemu_args(build["iso"],64,serial,trace,port)
    (out/"qemu-command.txt").write_text(" ".join(args),encoding="utf-8")
    p=subprocess.Popen(args,cwd=REPO,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE,text=True)
    s=f=None
    try:
        s,f=qmp_connect(port,p); qmp_cmd(f,"qmp_capabilities",ident=10)
        wait_serial(serial,"INPUT READY gsi=",p,30)
        entry_resets=read_shared(trace).count("CPU Reset")
        text=wait_serial(serial,"G4INPUT ERROR wait index=00 status=00000001",p,8)
        assert_true("G4 INPUT OBSERVED COMPLETE" not in text,"timeout case falsely completed")
        final_resets=assert_trace(trace,entry_resets)
        return {"opt":build["opt"],"ram":64,"kind":"timeout","serial_sha256":sha(serial),"trace_sha256":sha(trace),"entry_resets":entry_resets,"final_resets":final_resets}
    finally:
        if f:
            try:qmp_cmd(f,"quit",ident=99)
            except Exception:pass
        if s:
            try:s.close()
            except Exception:pass
        try:p.wait(timeout=3)
        except subprocess.TimeoutExpired:p.kill();p.wait()

def main():
    head=run(["git.exe","rev-parse","HEAD"],"HEAD").strip()
    assert_true(head==EXPECTED_HEAD,"unexpected HEAD")
    tag=run(["git.exe","rev-parse","v12.4.0^{}"],"frozen tag").strip()
    assert_true(tag==FROZEN_BASE_HEAD,"v12.4.0 tag drifted")
    source_guard()
    run(["cargo.exe","build","-q","-p","dbyte_cli","--bin","dbytec"],"fresh compiler")
    g3=run(["powershell.exe","-NoProfile","-ExecutionPolicy","Bypass","-File",str(REPO/"scripts/verify_v13_generation3.ps1")],"fresh G3 predecessor")
    (ROOT/"g3-predecessor.txt").write_text(g3,encoding="utf-8")
    epoch=int(run(["git.exe","show","-s","--format=%ct","HEAD"],"epoch").strip())
    date=time.strftime("%Y%m%d%H%M%S00",time.gmtime(epoch))
    manifest=source_manifest(); (ROOT/"input-manifest.json").write_text(json.dumps(manifest,indent=2),encoding="utf-8")
    builds=[]
    for opt in ("0","1"):
        a=build_set("A",opt,date); b=build_set("B",opt,date)
        assert_true(a["obj_sha256"]==b["obj_sha256"],f"O{opt} object nondeterministic")
        assert_true(a["elf_sha256"]==b["elf_sha256"],f"O{opt} ELF nondeterministic")
        assert_true(a["iso_sha256"]==b["iso_sha256"],f"O{opt} ISO nondeterministic")
        builds.extend([a,b])
    (ROOT/"builds.json").write_text(json.dumps(builds,indent=2),encoding="utf-8")
    receipts=[]
    for opt in ("0","1"):
        build=next(x for x in builds if x["label"]=="A" and x["opt"]==opt)
        for ram in (32,64): receipts.append(boot_positive(build,ram))
        receipts.append(boot_timeout(build))
    (ROOT/"qemu-receipts.json").write_text(json.dumps(receipts,indent=2),encoding="utf-8")
    selected=next(x for x in builds if x["label"]=="A" and x["opt"]=="1")
    result={"status":"PASS","generation":"v13-g4-observed-input-qemu","head":head,
            "compiler_sha256":sha(COMPILER),"manifest_sha256":sha(ROOT/"input-manifest.json"),
            "selected_object_sha256":selected["obj_sha256"],"selected_elf_sha256":selected["elf_sha256"],
            "selected_iso_sha256":selected["iso_sha256"],"selected_iso_bytes":selected["iso_bytes"],
            "qemu_receipts":len(receipts),"positive_receipts":sum(1 for x in receipts if x["kind"]=="positive"),
            "timeout_receipts":sum(1 for x in receipts if x["kind"]=="timeout"),"g3_predecessor":"PASS",
            "input_transport":"QMP input-send-event -> emulated PS/2 -> i8042 -> IOAPIC IRQ1",
            "guest_injection":"ABSENT","real_biostar":"PENDING","real_storage":"DISARMED","v13_final":"NOT_CREATED"}
    (ROOT/"RESULT.json").write_text(json.dumps(result,indent=2),encoding="utf-8")
    print("V13 GENERATION 4 OBSERVED INPUT VERIFIER: PASS")
    print("EVIDENCE ROOT:",ROOT)
    print("SELECTED ISO SHA256:",result["selected_iso_sha256"])
    print("QEMU RECEIPTS:",len(receipts))

if __name__=="__main__":
    try:
        main()
    except Exception as exc:
        print("V13 GENERATION 4 OBSERVED INPUT VERIFIER: FAIL")
        print("EVIDENCE ROOT:",ROOT)
        raise
