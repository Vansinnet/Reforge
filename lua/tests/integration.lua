-- End-to-end check: real LuaJIT ffi + real reforge.dll + real reforge.lua.
-- Run with the working directory set to <fake game>/binaries:
--   luajit integration.lua <path to reforge.lua> <sha256 of bundle/aaaa1111>
local lib_path, sha = arg[1], arg[2]
local passed, failed = 0, 0

local function check(name, ok, detail)
	if ok then passed = passed + 1; print("ok   " .. name)
	else failed = failed + 1; print("FAIL " .. name .. ": " .. tostring(detail)) end
end

local function read(path)
	local f = io.open(path, "rb")
	if not f then return nil end
	local s = f:read("*a")
	f:close()
	return s
end

_G.Mods = { lua = { ffi = require("ffi") } }

local function new_mod(name)
	local mod = { logs = {}, echoes = {}, errors = {} }
	function mod:get_name() return name end
	function mod:get_internal_data() return name end
	function mod:info(f, ...) self.logs[#self.logs + 1] = string.format(f, ...) end
	function mod:error(f, ...) self.errors[#self.errors + 1] = string.format(f, ...) end
	function mod:echo(f, ...) self.echoes[#self.echoes + 1] = string.format(f, ...) end
	function mod:command() end
	return mod
end

local reforge = dofile(lib_path)
local alpha = new_mod("Alpha")

check("stock before commit", read("../bundle/aaaa1111") == "STOCK-A")

local h = reforge.register(alpha, { stock = "bundle/aaaa1111", file = "payload/a.bin", sha256 = sha })
local v = reforge.register(alpha, { stock = "bundle/cccc3333", file = "payload/new.bin", virtual = true })
local bad = reforge.register(alpha, { stock = "bundle/bbbb2222", file = "payload/a.bin", sha256 = string.rep("0", 64) })
reforge.commit()

check("dll loaded", select(2, reforge.version()) ~= nil, alpha.errors[1])
-- Opens made before reforge.dll was installed cannot be seen (documented limitation).
check("file opened before install reports active", reforge.state(h) == "active", reforge.state(h))
check("served after commit", read("../bundle/aaaa1111") == "MOD-A", read("../bundle/aaaa1111"))
local beta = new_mod("Beta")
local hb = reforge.register(beta, { stock = "bundle/aaaa1111", file = "payload/b.bin", sha256 = sha, priority = 1 })
check("winner change after an open needs restart", reforge.state(hb) == "restart_required", reforge.state(hb))
check("new winner served to later opens", read("../bundle/aaaa1111") == "MOD-B", read("../bundle/aaaa1111"))
reforge.clear(beta)
check("virtual file active and served", reforge.state(v) == "active" and read("../bundle/cccc3333") == "NEW")
check("wrong sha refused by the dll", reforge.state(bad) == "refused" and read("../bundle/bbbb2222") == "STOCK-B", reforge.reason(bad))
reforge.set_enabled(false)
check("set_enabled(false) serves stock", read("../bundle/aaaa1111") == "STOCK-A")
reforge.set_enabled(true)
reforge.clear(alpha)
check("clear serves stock", read("../bundle/aaaa1111") == "STOCK-A" and read("../bundle/cccc3333") == nil)

print(string.format("\n%d passed, %d failed", passed, failed))
os.exit(failed == 0 and 0 or 1)
