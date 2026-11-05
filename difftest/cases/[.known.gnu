# GNU test has no grammar for more than four arguments: nested parentheses
# around a string are a syntax error for it, an answer for the spec rule and for us
$(yes '(' | head -100) x $(yes ')' | head -100) ']'
