// RTI Bridge — newline-delimited JSON over TCP (see docs/oracle.md).
//
// Implemented: hello, ping, state, load_map, capture.
// Not implemented: run (input playback). Openplanet has no supported input
// injection; that needs TMInterface or a TAS tool. `run` returns an error
// that says so, so RTI can fall back to capture-based verification.

const uint PROTOCOL = 1;
const uint16 PORT = 27015;

Net::Socket@ g_server = null;
array<Net::Socket@> g_clients;
array<string> g_buffers;

// capture state
bool g_capturing = false;
uint g_captureStartTime = 0;
uint g_captureMaxTicks = 0;
array<string> g_captureRows;
Net::Socket@ g_captureClient = null;

void Main() {
    @g_server = Net::Socket();
    if (!g_server.Listen("127.0.0.1", PORT)) {
        warn("[RTI] could not listen on port " + PORT);
        return;
    }
    print("[RTI] bridge listening on 127.0.0.1:" + PORT);
    while (true) {
        Net::Socket@ c = g_server.Accept();
        if (c !is null) {
            g_clients.InsertLast(c);
            g_buffers.InsertLast("");
            print("[RTI] client connected");
        }
        for (int i = int(g_clients.Length) - 1; i >= 0; i--) {
            Net::Socket@ s = g_clients[i];
            if (!s.IsReady() || s.IsHungUp()) {
                g_clients.RemoveAt(i);
                g_buffers.RemoveAt(i);
                continue;
            }
            int avail = s.Available();
            if (avail > 0) {
                g_buffers[i] += s.ReadRaw(avail);
                int nl = g_buffers[i].IndexOf("\n");
                while (nl >= 0) {
                    string line = g_buffers[i].SubStr(0, nl);
                    g_buffers[i] = g_buffers[i].SubStr(nl + 1);
                    Handle(s, line.Trim());
                    nl = g_buffers[i].IndexOf("\n");
                }
            }
        }
        if (g_capturing) StepCapture();
        yield();
    }
}

void Send(Net::Socket@ s, const string &in json) {
    s.WriteRaw(json + "\n");
}

string Err(const string &in msg) {
    Json::Value v = Json::Object();
    v["ok"] = false;
    v["error"] = msg;
    return Json::Write(v);
}

CSceneVehicleVisState@ CarState() {
    auto vis = VehicleState::ViewingPlayerState();
    return vis;
}

Json::Value StateJson(uint tick) {
    Json::Value v = Json::Object();
    auto vis = CarState();
    v["tick"] = tick;
    if (vis is null) {
        v["available"] = false;
        return v;
    }
    v["available"] = true;
    Json::Value pos = Json::Array(); pos.Add(vis.Position.x); pos.Add(vis.Position.y); pos.Add(vis.Position.z);
    Json::Value vel = Json::Array(); vel.Add(vis.WorldVel.x); vel.Add(vis.WorldVel.y); vel.Add(vis.WorldVel.z);
    v["pos"] = pos;
    v["vel"] = vel;
    // heading in the x/z plane from the car's forward vector
    v["yaw"] = Math::Atan2(vis.Dir.z, vis.Dir.x);
    v["speed_kmh"] = vis.FrontSpeed * 3.6f;
    auto app = cast<CTrackMania>(GetApp());
    auto playground = cast<CSmArenaClient>(app.CurrentPlayground);
    uint cp = 0; bool finished = false; uint raceTime = 0;
    if (playground !is null && playground.GameTerminals.Length > 0) {
        auto player = cast<CSmPlayer>(playground.GameTerminals[0].ControlledPlayer);
        if (player !is null) {
            auto script = cast<CSmScriptPlayer>(player.ScriptAPI);
            if (script !is null) {
                raceTime = script.CurrentRaceTime > 0 ? uint(script.CurrentRaceTime) : 0;
                cp = script.RaceWaypointTimes.Length;
            }
        }
    }
    v["cp"] = cp;
    v["finished"] = finished;
    v["race_time_ms"] = raceTime;
    return v;
}

void Handle(Net::Socket@ s, const string &in line) {
    if (line.Length == 0) return;
    Json::Value req = Json::Parse(line);
    if (req.GetType() != Json::Type::Object) { Send(s, Err("bad json")); return; }
    string cmd = req["cmd"];
    Json::Value resp = Json::Object();
    resp["ok"] = true;
    if (cmd == "hello") {
        resp["protocol"] = PROTOCOL;
        resp["game"] = "TM2020";
        resp["capabilities"] = "hello,ping,state,load_map,capture";
        Send(s, Json::Write(resp));
    } else if (cmd == "ping") {
        Send(s, Json::Write(resp));
    } else if (cmd == "state") {
        resp["state"] = StateJson(0);
        Send(s, Json::Write(resp));
    } else if (cmd == "load_map") {
        // Plays a map file from the user's Maps folder (Documents/Trackmania/Maps),
        // e.g. "RTI/tmx356566.Map.Gbx". UIDs are not resolvable offline; RTI
        // writes the .Map.Gbx it downloaded and passes the relative path in "file".
        string file = req["file"].GetType() == Json::Type::String ? string(req["file"]) : "";
        if (file.Length == 0) { Send(s, Err("load_map needs \"file\" (path relative to the Maps folder)")); return; }
        auto app = cast<CTrackMania>(GetApp());
        app.BackToMainMenu();
        while (!app.ManiaTitleControlScriptAPI.IsReady) yield();
        app.ManiaTitleControlScriptAPI.PlayMap(file, "TrackMania/TM_PlayMap_Local", "");
        // wait for the playground
        uint t0 = Time::Now;
        while (app.CurrentPlayground is null && Time::Now - t0 < 30000) yield();
        if (app.CurrentPlayground is null) { Send(s, Err("map did not load within 30 s: " + file)); return; }
        Send(s, Json::Write(resp));
    } else if (cmd == "capture") {
        // Stream telemetry of whatever is driving (human, ghost) for max_ticks
        // ticks, then reply with {ok, states:[...]}. One capture at a time.
        if (g_capturing) { Send(s, Err("capture already running")); return; }
        g_captureMaxTicks = uint(req["max_ticks"]);
        if (g_captureMaxTicks == 0) g_captureMaxTicks = 12000;
        g_captureRows.Resize(0);
        g_captureStartTime = Time::Now;
        @g_captureClient = s;
        g_capturing = true;
    } else if (cmd == "run") {
        Send(s, Err("run (input playback) is not available in this bridge; Openplanet cannot inject inputs. Use TMInterface/TAS tooling for replay validation, or capture-based verification. See docs/oracle.md"));
    } else {
        Send(s, Err("unknown cmd " + cmd));
    }
}

void StepCapture() {
    uint elapsed = Time::Now - g_captureStartTime;
    uint tick = elapsed / 10;
    Json::Value st = StateJson(tick);
    g_captureRows.InsertLast(Json::Write(st));
    if (tick >= g_captureMaxTicks || g_captureRows.Length > 200000) {
        string out = "{\"ok\":true,\"result\":{\"finished\":false,\"race_time_ms\":" + elapsed + ",\"ticks\":" + tick + ",\"checkpoints\":0},\"states\":[";
        for (uint i = 0; i < g_captureRows.Length; i++) {
            if (i > 0) out += ",";
            out += g_captureRows[i];
        }
        out += "]}";
        if (g_captureClient !is null) Send(g_captureClient, out);
        g_capturing = false;
        @g_captureClient = null;
        g_captureRows.Resize(0);
    }
}
