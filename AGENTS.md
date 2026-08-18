- Don't run `cargo sqlx prepare`, the human will do that before committing.
- use sqlx macros for database queries
- To update the schema: write to files under `schema/xx-<name>.sql` in the
  crate. Run `create-migration.sh initial` after removing the
  `migrations/xxx_initial.sql`, then `sqlx database reset -fy`.
- Use the error handling in `minilith-errors` (NOT color_eyre), except for
  functions which are called in `Context::new` in the respective folders
- We have an alert system set up. If something warrants sending an email use it.
  For internal errors, prefixing the message with `l1` or `l2` makes the error
  handling send an alert of level 1 respectively level 2 instead of the default
  level 3. In general the higher the level the more recipients and the higher
  criticality. Level 1 is generally for loss of funds for us or customer, level
  2 for service breaking signs, and level 3 for potential behaviour breakage.
- run `cargo clippy` to fix all lints
- keep the documentation amount low, just document the non-obvious necessary
  things
- alert the human if anything is ambiguous
- keep down code duplication
- the goal of the backend is to be maintainable easily by future developers. Do
  extra work to make a plan of the best way to approach the problem before
  beginning to code.
- follow the vision and goals outlined in `../docs/krav.typ`,
  `../docs/vision-och-värdegrund.typ`, `../docs/beslut/beslutsmetod.typ`
- always give feedback on the prompt and ideas the human presents in relation to
  the vision and goals outlined above
- a list of our technologies can be found in `../docs/beslut/teknologier/`,
  including example motivations of how to make decisions
- change visibility for existing structs & similar to reuse code. It's OK to
  make everything public since this is internal code
- write any contracts in the endpoint documentation
- if anything is against good practice or could be made better, please notify
  the human
- if anything can be refactored to clean up the code and understanding when
  reading the code, notify the human

You are not writing code to make something work, you are writing the code to
make the project more maintainable.

After compacting the context, re-read this document and the documents mentioned
above so they stay in their full in your context.
