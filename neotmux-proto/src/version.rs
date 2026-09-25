/// Bumped whenever any type here changes shape; checked before the first frame decodes so a
/// version mismatch fails outright instead of trying to survive one.
///
/// Every bump records what changed and whether a renderer needed it. Three bumps no renderer
/// needed is the spec's trigger (§2.1) for giving `DrawCmd` its own mirror in the codec, and
/// that count is only checkable if it is written down here.
///
/// The record stopped being kept after 3 and the nine bumps to 12 went unwritten, so the
/// count the trigger needs starts again at 12 rather than pretending to reach back.
///
/// - 1: initial version.
/// - 2: the frame stopped being the only thing on the wire. A tick, its reply, the ops, the
///   events, the geometry and the pane reads all cross now, because the Lua layer runs in
///   the client (§11, M3). A renderer needed every one of them.
/// - 3: `SessionOp::Detach` and `End::Detach`, so a client can leave without ending the
///   session. A renderer needed both.
/// - 4 to 11: not recorded at the time, and not reconstructed here — a record written after
///   the fact is not one this rule can be trusted to.
/// - 12: `Read` and `Reading` left the wire. The Lua layer runs in the server, so the pane
///   read it used to ask across the socket is a function call and nothing sent either
///   message. A renderer needed neither: no renderer ever spoke them.
/// - 13: `DrawCmd::Text` carries `bold`, a byte after its colours. A renderer needed it: it is
///   an attribute the renderer draws.
/// - 14: `Tick` and `Tock` left the wire, and `Done` stopped carrying steps, which went empty
///   the day the Lua layer moved into the server. A renderer needed none of them: the client
///   had ignored all three since.
pub const PROTOCOL_VERSION: u32 = 14;
