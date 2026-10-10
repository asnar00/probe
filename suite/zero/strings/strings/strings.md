# strings
*a string is a value: `s == t` on two strings is one `bool`*

layer: runtime

> (suite) 2026-10-10T19:00:00
fm3 question 100, decided by question 126's sixth principle (a utility type is declared in zero with only the functions that make sense), log 242.

## overview
`string s` has no mark: it is a value made of characters and not an array, so `==` on two strings does not go character by character and give a `bool` for each. `s == t` is one `bool`, are the two texts the same, and `s != t` its opposite. `if (name == "zero")` is the first line anyone writes.

It is declared in zero, in the language's own feature, on the model of a `time`'s operators:

    on (bool r) << (string a) == (string b)
        r << a [==] b

and the compiler writes that one line where the comparison stands. `s [==] t`, the whole-array comparison it is said in, goes on working.

## interface
- `same (k)` compares three names; `shorter` a text with a longer one that begins the same; `written first` a text written out on the left; `both written` two texts written out.
- `kept is zero` compares a string at feature scope; `is zero (s)` a parameter, and `handed (k)` calls it twice.
- `tested` uses the comparison as the condition of an `if`.
- `words (k)` compares structures that have a `string` field, and `a word given (k)` one with a structure made where it is compared.
- `by hand` says `[==]`.

## rules
- **Two strings the same**: the same length and the same characters in order. `shorter` is 0.
- **A string is told from an array of characters by how it is written**: a name with no mark whose type is `string`, or a text in quotes. `cs[] == ds[]` on two arrays of `char` is still each pair, and refused where one `bool` is wanted, naming `[==]`.
- **Only what makes sense is declared**: `s < t` is refused, "`<` on two strings: a string has `==` and `!=`, whether two texts are the same, and nothing that says which comes first".
- **A structure with a `string` field compares field by field** (fm3 question 108, which refused it for want of this): `words (1)` finds `a == b` and `b != c`.
- **What it costs**: what `s [==] t` costs, to the operation; `by hand` and a comparison by `==` of the same two names emit the same lines.

## testing
>same (1) → 101
>shorter() → 0
>written first() → 1
>both written() → 10
>kept is zero() → 1
>handed (1) → 10
>tested() → "it is"
>words (1) → 101
>words (2) → 1
>a word given (1) → 1
>a word given (3) → 0
>by hand() → 1

## hostile
Not built, each still refused as two arrays compared, and each said by hand with `[==]`: a string that is a structure's field read by name, `a.text == "let"`, and one a function gives, `name of (k) == "one"` (fm3 question 144). The compiler's type for a string is an array of characters', and only a name or a text in quotes shows which it is.
