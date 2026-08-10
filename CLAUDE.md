# CLAUDE.md

This file gives the rules for all work in this repository. Obey each rule.

1. Write all text that is not code in ASD-STE100 Simplified Technical English.
   This rule applies to commit messages, documentation, and conversation
   responses. Do not forget this rule at any time.

2. Do not use negative parallelism. Do not use announcer phrases. Do not use
   metaphorical language.

3. Write each commit message as prose paragraphs. Give the technical content of
   the change. Give the reason for the change. Do not add opinions. Do not add
   other details.

4. Make the unit tests cover 100% of the lines, the branches, and the functions.

5. Commit directly to the `main` branch. Do not use feature branches.

6. Use a pre-commit hook to run the unit tests and the linters. The hook must
   stop a commit that does not pass.

7. Make a plan before you change the code. Then examine your plan for errors.
   Then make the planned change. Then examine your work for errors.
