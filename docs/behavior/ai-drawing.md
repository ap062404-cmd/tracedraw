# AI Drawing

The AI Generate docker's Draw with words mode (its fourth mode button; also Object > AI Generate > Draw with Words and the arrow of the toolbar's AI Generate button) shows a panel
that turns a request in words ("a red bicycle with blue wheels", "make
the circle bigger and put the title under it") into a drawing. An AI
service of the user's choice, with the user's own key, writes a program
for the scripting object model (the one of the Scripts panel), and
TraceDraw runs it as one undo step. TraceDraw has no AI of its own and no
account; nothing is sent until the user presses Draw.

## Prompt bar

While a service is set up (a model, and a key where the service needs
one; a chat completions service at an address of the user's needs no
key), a bar shows above the colour palette, as wide as the drawing
window: "AI", the model list, the request (Enter draws, like Draw), Draw
(a spinner and Cancel while waiting), what the last request did, `...`
(opens AI Generate in Draw with words) and `x` (hides the bar). Window > Toolbars >
AI Prompt Bar and "Show the prompt bar above the palette" in the panel
turn it on and off (on by default); it does not show on the Welcome
Screen. The request is shared with the panel's, and a request that drew
clears it.

The model list offers the service's own models once fetched ("Fetch the
service's models": a GET of `models` next to the address,
`/v1/models?limit=1000` for the Messages API so every page of models
comes, `data[].id` of the reply without a leading `models/`), before
that the usual ones (`claude-sonnet-5-5`, `claude-opus-5-5`,
`claude-haiku-5-5`, `claude-fable-5-1` for the Messages API, none for
chat completions); the model in use, and the image model set up in the
image model lists, are always listed, first when the service's list does
not hold them (`ai::model_choices`).

## Connection

The connection is the one AI service, also set in Tools > Options > AI >
Service (see `ai-options.md`), whose key also serves pictures when the
service can make them; with AI features turned off there, Draw and the
prompt bar are off and the panel shows a banner. For a service that can
make pictures (OpenAI, Google Gemini, a custom OpenAI-compatible
address) the connection also has an Image model row; for one that makes
drawings only, a line says so.

The Connection section is open until a key is entered for a service that
needs one. Its rows follow the room there is (#2040,
`ui::ai_docker::Rows`): the labels stand in a column as wide as the
longest of them and each field takes what is left, from 110 points up to
360 (so a long address shows in Tools > Options > AI and the fields of a
narrow docker are not cut off, as the fixed 170 points were); in a panel
too narrow for a label and a field of 110 points side by side, each
label goes above its field, which is then as wide as the panel. The
Preset and Service lists cut a name that does not fit with an ellipsis.
Under the request, Draw and New conversation wrap onto two lines in a
narrow panel.

| Field | Default | Notes |
|---|---|---|
| Service | Anthropic (Messages API) | or OpenAI-compatible (chat completions), the API many services and local model servers offer |
| Address | `https://api.anthropic.com/v1/messages`, or `https://api.openai.com/v1/chat/completions` | empty uses the default shown in grey; local servers: their own address, such as `http://localhost:11434/v1/chat/completions`. An address of the server alone (`http://localhost:8787`) gets `/v1/messages` or `/v1/chat/completions`, one ending in `/v1` gets `/messages` or `/chat/completions`; any other path is used as typed (#1011, `ai::tests::an_address_without_its_path_gets_the_api_path`) |
| Model | `claude-sonnet-5-5` for the Messages API; none for chat completions | the model name as the service lists it |
| Longest reply | 32000 tokens | Messages API only (the row shows for it): `max_tokens` of each request, 1000 to 128000; a reply cut short at it says so. 8000 before #1011 cut long drawings short (`ai::tests::the_longest_reply_is_a_preference`) |
| API key | empty | a password field; needed by the Messages API, optional for chat completions (sent as `Authorization: Bearer`) |
| Remember the key on this computer | off | on: the key is kept in the preferences file (in the browser: the page's local storage), in plain text; off: it lives only until TraceDraw closes |

Choosing another service clears the address and model (their defaults
apply).

The Model list (the arrow beside the field, and the list on the prompt
bar) offers the usual models of the service until "Fetch the service's
models" asks for its own list. That list holds every model the service
has, and most services list models that cannot draw: embedding, audio
and speech, live, robotics, research, question answering, music, video
and image models, which only fail when picked. The list therefore
offers the models that can draw (#2039, `ai::drawing_models`): a model
stays out when a whole word of its name says what else it is for
(`embedding`, `audio`, `tts`, `live`, `realtime`, `robotics`, `aqa`,
`lyria`, `veo`, `imagen`, `image`, `whisper`, `moderation` and the like,
or `deep-research` and `computer-use`), since the services say no more
about a model than its name; the finished models come before the ones
on trial (`preview`, `exp`, `beta`), each group in the service's order.
The model set up always stays in the list, first when the list would
leave it out. "Show every model the service lists", a box in the list
shown when something was left out, brings them all back until TraceDraw
closes. A list whose names tell nothing apart (a server with names of
its own) is offered whole.

## Drawing

- Write the request and press Draw (or Ctrl+Enter in the request).
  While waiting the panel shows a spinner and Cancel (the reply is then
  dropped when it comes).
- What is sent: the instructions and a reference of the object model
  (`ai::SYSTEM_PROMPT`: units in millimetres, origin at the page's
  bottom-left corner with y up, angles counter-clockwise in degrees, text
  sizes in points, colours, every method the program may use); up to
  the 3 earlier requests of the conversation with their replies; the
  request with a JSON description of the active page: its size, its
  layers (the active one marked) and up to 300 objects (id, type, name,
  bounds in mm, fill, outline, opacity, text and font, group children;
  the number of objects left out), and the selection. File names,
  pictures and the other pages are not sent.
- Messages API requests carry `x-api-key`, `anthropic-version:
  2023-06-01`, `max_tokens` (Longest reply, 32000 at first) and the instructions as `system`; chat
  completions requests put the instructions first as a `system` message.
- The reply's program is its JavaScript code block (else its first code
  block, else the reply itself when it reads as code). It runs as an
  untrusted script: Export and ImportBitmap are refused (no file access),
  each function call may run at most 1,000,000 loop iterations, calls
  nest at most 400 deep and the program may make at most 20,000 changes.
  It is all or nothing: a program that fails leaves the drawing as it
  was. Its changes are one undo step, "AI Drawing".
- A program that fails is sent back once, with its error, asking for the
  whole corrected program ("The script failed; asking for a corrected
  one..."). If that one fails too, nothing is drawn and the error shows.
- After a drawing, the panel says how many changes were made, shows what
  the program printed, and keeps the program under Script, where Open in
  Scripts copies it to the Scripts panel for editing and running again.
- New conversation forgets the earlier requests.
- A service that is busy for the moment is asked again by itself
  (`ai_retry.rs`, #2037; before, the error showed at once and the only
  way on was Draw again or another model): after 500, 502, 503 or 529
  the same request goes again after 2, 4 and 8 seconds, or after the
  wait the service's `Retry-After` asks for when that is 30 seconds or
  less, four tries in all; after 429 only when the service names such a
  wait (without one it is most often a used up quota, which seconds do
  not bring back). Meanwhile the panel and the prompt bar say "The
  service is busy at the moment (503). Trying again, attempt 2 of 4...",
  and Cancel stops it. The waiting happens between frames, so the window
  stays live, the same on the desktop and in a browser. A service still
  busy after the last try is said as such, with what to do (wait a
  little, or choose another model) and its own message. AI Generate,
  Remove Background and the model lists are sent the same way.
- Errors shown, said plainly and followed by the service's own message
  (`ai::http_error`, also for AI Generate and Remove Background): 429 as
  a used up quota or credit or too many requests, with the wait the
  service's `Retry-After` asks for (seconds or a date); 401 and 403 as a
  key the service refuses; 408, 504 and 524 as a service that took too
  long; 502, 503 and 529 as a busy service (with its wait); 404 as an
  address or model it does not know; any other status with the service's
  message as it is. A request that gets no answer in five minutes says
  the service took too long and to try again; other network failures
  say the service could not be reached and why. Also a reply cut short
  (ask for less at a time), a reply with no program, a program over
  256 KB.
- The program can also use `ActiveLayer.CreatePath(d)`: SVG path data
  (`M L H V C S Q T A Z`) in page coordinates, which the Scripts panel
  has too.

## Desktop and browser

The desktop program sends the request over HTTPS, checking certificates
against the system's trusted ones and using the proxy of the usual
environment variables (`HTTPS_PROXY`, `NO_PROXY`), and waits up to five
minutes. In the browser the page sends the request itself, so the
service must accept requests from web pages: the Messages API does when
told the key is the user's own (the request carries
`anthropic-dangerous-direct-browser-access: true`); a local server must
allow the page's origin (for example `OLLAMA_ORIGINS`).

## Checks

- A request to a test transport draws a red circle as one undo step,
  the page goes along, and the next request carries the earlier one
  without its page (`ai::tests::a_request_draws_through_the_service_as_one_undo_step`).
- The model lists offer the models that can draw, the finished ones
  first, keep the model set up, and show every model on request; names
  decide, by whole words
  (`ai::tests::the_model_list_offers_the_models_that_can_draw`,
  `every_model_can_be_shown_and_names_decide_which_draw`).
- A program that draws then fails is sent back with its error, and only
  the corrected drawing stays; a program that fails twice draws nothing
  (`ai::tests::a_failing_script_is_sent_back_once_and_nothing_half_drawn_stays`).
- Export and ImportBitmap are refused and an endless loop stops; the
  user's own scripts still export
  (`ai::tests::scripts_from_the_service_cannot_reach_files_or_run_forever`).
- Requests for both kinds of service carry their headers, model and
  messages (`ai::tests::requests_are_built_for_both_kinds_of_service`);
  replies, error replies, cut and empty replies are read
  (`ai::tests::replies_and_errors_are_read`); code blocks are found
  (`ai::tests::the_program_is_taken_from_its_code_block`).
- No request leaves without words, a key (where needed) and a model
  (`ai::tests::requests_need_words_a_key_and_a_model`); network and
  service errors are shown (`ai::tests::errors_of_the_service_and_the_network_are_shown`);
  the plain messages for 429 (with its wait), the key, timeouts, a busy
  service and an unknown model (`ai::tests::error_replies_say_what_went_wrong`,
  `requests_that_fail_say_why`); every model set up stays in the lists
  (`ai::tests::model_lists_hold_every_model_set_up`).
- A busy service is asked again with growing waits, the service's own
  wait is followed when short, and other answers are never sent again
  (`ai_retry::tests`); a drawing comes after two busy answers, a service
  that stays busy gets four tries and a plain message, also for the model
  list and for pictures
  (`ai::tests::a_busy_service_is_asked_again_until_it_draws`,
  `a_service_that_stays_busy_ends_in_a_clear_message`,
  `ai_image::tests::a_busy_image_service_is_asked_again`).
- The desktop transport posts to a server on the same computer and reads
  its error replies (`ai::tests::the_desktop_transport_posts_and_reads_error_replies`);
  the real Messages API refuses a made-up key with 401
  (`ai::tests::the_real_service_refuses_a_made_up_key`, run by hand with
  `--ignored`).
- Every method the reference names exists in the object model
  (`ai::tests::the_reference_names_only_methods_the_object_model_has`).
- The key is written to the preferences only when it is to be
  remembered (`ui::ai_docker::tests::the_key_is_kept_only_when_asked`);
  the panel draws in every state (`ui::ai_docker::tests::the_docker_draws_in_every_state`).
- The active page's description lists up to the limit and counts the
  rest (`describe::tests::the_active_page_is_described_up_to_a_number_of_objects`).
- The prompt bar shows only with a service set up and the bar on, and
  the window draws with it in every state
  (`ui::ai_bar::tests::the_bar_shows_while_a_service_is_set_up`); the
  model list takes the service's models and keeps the one in use; the
  models address of both kinds of service
  (`ui::ai_bar::tests::the_model_list_offers_the_services_models`).
- Checked on screen: a request typed in the bar and Enter drew a sun, a
  hill with a gradient and a title from a test service on the same
  computer, and cleared the field.
