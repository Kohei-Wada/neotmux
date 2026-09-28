//! `neotmux.pane.content(id, { runs = true })`: each row as the runs it is drawn in, with
//! colours in the encoding the draw verbs take, so a colour read can be drawn back.

mod support;

use support::{built, feed, keys, logged};

/// Every run of the first row, one log line each: `text|fg|bg|attributes`.
const SAY_RUNS: &str = "neotmux.on_key(function(key)\n\
      if key ~= 'J' then return false end\n\
      local rows = neotmux.pane.content(1, { runs = true })\n\
      for _, run in ipairs(rows[1]) do\n\
        local on = {}\n\
        for _, name in ipairs({ 'bold', 'dim', 'italic', 'underline', 'inverse' }) do\n\
          if run[name] then on[#on + 1] = name end\n\
        end\n\
        neotmux.log.info(run.text .. '|' .. tostring(run.fg) .. '|' .. tostring(run.bg) .. '|' .. table.concat(on, ','))\n\
      end\n\
      neotmux.log.info('rows ' .. #rows)\n\
      return true\n\
    end)\n";

#[test]
fn a_row_is_read_as_runs_with_the_colours_the_draw_verbs_take() {
    let (_dir, sub) = built(SAY_RUNS);
    feed(
        &sub,
        b"say \x1b[1;31mred\x1b[0m and \x1b[38;2;255;128;0;4morange\x1b[0m\x1b[44m  \x1b[0m",
    );
    keys(&sub, &["J"]);
    let said = logged(&sub);
    assert_eq!(
        said[..5],
        [
            "say |nil|nil|".to_string(),
            "red|1|nil|bold".to_string(),
            " and |nil|nil|".to_string(),
            "orange|#ff8000|nil|underline".to_string(),
            "  |nil|4|".to_string(),
        ],
        "{said:?}"
    );
}

/// Without `runs`, the answer is the lines it always was.
#[test]
fn without_runs_a_row_is_still_a_line_of_text() {
    let (_dir, sub) = built(
        "neotmux.on_key(function(key)\n\
           if key ~= 'J' then return false end\n\
           neotmux.log.info(neotmux.pane.content(1)[1])\n\
           return true\n\
         end)\n",
    );
    feed(&sub, b"plain \x1b[31mred\x1b[0m");
    keys(&sub, &["J"]);
    assert_eq!(logged(&sub), vec!["plain red".to_string()]);
}
