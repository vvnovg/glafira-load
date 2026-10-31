package demo

import org.springframework.web.bind.annotation.*

@RestController
@RequestMapping("/v1/payments")
class PaymentController {

    @PostMapping
    fun create(@RequestBody req: PaymentRequest): PaymentDto = TODO()

    @GetMapping("/{id}")
    fun get(@PathVariable id: String): PaymentDto = TODO()
}
