;; Adapted from tree-sitter-smali queries (MIT, Yotam Nachum, Amaan Qureshi); capture names mapped to Zed's.
; Types

(class_identifier
  (identifier) @type)

(primitive_type) @type

((class_identifier
   . (identifier) @_first @type
   (identifier) @type)
  (#any-of? @_first "android" "dalvik" "java" "kotlinx"))

((class_identifier
   . (identifier) @_first @type
   .  (identifier) @_second @type
   (identifier) @type)
  (#eq? @_first "com")
  (#any-of? @_second "android" "google"))

; Methods

(method_definition
  (method_signature (method_identifier) @function))

(expression
  (opcode) @_invoke
	(body
	  (full_method_signature
      (method_signature (method_identifier) @function)))
  (#match? @_invoke "^invoke"))

(method_handle
  (full_method_signature
	(method_signature (method_identifier) @function)))

(custom_invoke
  . (identifier) @function
  (method_signature (method_identifier) @function))

(annotation_value
  (body
    (method_signature (method_identifier) @function)))

(annotation_value
  (body
    (full_method_signature
      (method_signature (method_identifier) @function))))

(field_definition
	(body
		(method_signature (method_identifier) @function)))

(field_definition
	(body
	  (full_method_signature
		  (method_signature (method_identifier) @function))))

((method_identifier) @constructor
  (#any-of? @constructor "<init>" "<clinit>"))

"constructor" @constructor

; Fields

[
  (field_identifier)
  (annotation_key)
] @field

((field_identifier) @constant
  (#match? @constant "^[A-Z_]*$"))

; Variables

(variable) @variable.special

(local_directive
  (identifier) @variable)

; Parameters

(parameter) @variable.special
(param_identifier) @variable.special

; Labels

[
  (label)
  (jmp_label)
] @label

; Operators

(opcode) @keyword

((opcode) @keyword
  (#match? @keyword "^return"))

((opcode) @keyword
  (#match? @keyword "^if"))

((opcode) @keyword
  (#match? @keyword "^cmp"))

((opcode) @keyword
  (#match? @keyword "^throw"))

((opcode) @comment
  (#eq? @comment "nop")) ; haha, anyone get it? ;)

[
  "="
  ".."
] @operator

; Keywords

[
  ".class"
  ".super"
  ".implements"
  ".field"
  ".end field"
  ".annotation"
  ".end annotation"
  ".subannotation"
  ".end subannotation"
  ".param"
  (end_param_directive)
  ".parameter"
  ".end parameter"
  ".line"
  ".locals"
  ".local"
  ".end local"
  ".restart local"
  ".registers"
  ".packed-switch"
  ".end packed-switch"
  ".sparse-switch"
  ".end sparse-switch"
  ".array-data"
  ".end array-data"
  ".enum"
  (prologue_directive)
  (epilogue_directive)
] @keyword

[
  ".source"
] @include

[
  ".method"
  ".end method"
] @keyword

[
  ".catch"
  ".catchall"
] @keyword

; Literals

(string) @string
(source_directive (string "\"" _ @string.special "\""))
(escape_sequence) @string.escape

(character) @character

"L" @character.special

(number) @number

[
 (float)
 (NaN)
 (Infinity)
] @float

(boolean) @boolean

(null) @constant.builtin

; Misc

(annotation_visibility) @keyword

(access_modifier) @keyword

(array_type
  "[" @punctuation.special)

["{" "}"] @punctuation.bracket

["(" ")"] @punctuation.bracket

[
  "->"
  ","
  ":"
  ";"
  "@"
  "/"
] @punctuation.delimiter

(line_directive (number) @text.underline @string)

; Comments

(comment) @comment 

(class_definition
  (comment) @comment.documentation)

; Errors

(ERROR) @error
